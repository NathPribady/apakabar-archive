//! The reading room: browse, search and read every recovered message.
//!
//! Readers do not see messages that are restricted to researchers: those the editors marked
//! and the May 1998 messages no editor has reviewed yet (`messages.sensitive`, set at start-up
//! from the site store). Email addresses in the texts are masked for readers.

use super::auth::{Reader, User};
use super::charts;
use super::i18n::{self, lang_label, month_short, t, tf, type_label};
use super::layout::{csrf_field, fmt_n, nice_date, page};
use super::{blocking, site, AppError, AppState};
use crate::{lexicon, sources, text};
use axum::{
    extract::{Form, Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use maud::{html, Markup, PreEscaped};
use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::{params, params_from_iter, OptionalExtension};
use serde::Deserialize;

const DAILY_LIMIT: i64 = 1500;

#[derive(Clone)]
pub struct Row {
    pub ak_id: String,
    pub date: Option<String>,
    pub title: String,
    pub code: Option<String>,
    pub source_name: Option<String>,
    pub lang: String,
    pub mtype: String,
    pub words: i64,
    pub snippet: Option<String>,
    pub dup: bool,
    pub title_source: Option<String>,
    pub topic: Option<String>,
    pub restricted: bool,
}

const ROW_COLS: &str = "m.ak_id, m.date, m.title, m.source_code, m.source_name, m.lang, m.msg_type, m.words, m.dup_of IS NOT NULL, m.title_source, m.topic, m.sensitive";

fn row(r: &rusqlite::Row, snippet: bool) -> rusqlite::Result<Row> {
    Ok(Row {
        ak_id: r.get(0)?,
        date: r.get(1)?,
        title: r.get(2)?,
        code: r.get(3)?,
        source_name: r.get(4)?,
        lang: r.get(5)?,
        mtype: r.get(6)?,
        words: r.get(7)?,
        dup: r.get(8)?,
        title_source: r.get(9)?,
        topic: r.get(10)?,
        restricted: r.get::<_, i64>(11)? == 1,
        snippet: if snippet { r.get(12)? } else { None },
    })
}

/// The SQL condition that hides restricted messages from readers.
fn visible(u: &User) -> &'static str {
    if u.can_see_sensitive() { "1 = 1" } else { "m.sensitive = 0" }
}

// ---------------------------------------------------------------- privacy

static EMAIL: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b([a-z0-9][a-z0-9._%+-]*)@([a-z0-9-]+(?:\.[a-z0-9-]+)+)").unwrap());

/// Hide the personal part of email addresses ("•••@igc.apc.org"). The list's own addresses
/// (apakabar@…, indonesia-l@…) stay, since they are part of the record.
pub fn mask_emails(s: &str) -> String {
    EMAIL
        .replace_all(s, |c: &regex::Captures| {
            let local = c[1].to_ascii_lowercase();
            if local.starts_with("apakabar") || local.starts_with("indonesia-") || local.starts_with("reg.") || local == "majordomo" || local == "listserv" {
                c[0].to_string()
            } else {
                format!("\u{2022}\u{2022}\u{2022}@{}", &c[2])
            }
        })
        .into_owned()
}

/// What a viewer may read of a text: everything for researchers and editors, addresses
/// masked for readers.
fn shown(u: &User, s: &str) -> String {
    if u.can_see_sensitive() { s.to_string() } else { mask_emails(s) }
}

/// How a title was made, for titles that are not the subject line.
pub fn title_note(src: &str) -> Option<&'static str> {
    match src {
        "inner_subject" => Some(t("Title from the Subject line inside the message", "Judul diambil dari baris Subject di dalam pesan")),
        "body_prefix" => Some(t("The subject was cut short; the full title is the first line of the text", "Subjeknya terpotong; judul lengkapnya adalah baris pertama teks")),
        "headline" => Some(t("Title from the headline at the top of the text; the subject line names the conference topic", "Judul diambil dari kepala berita di awal teks; baris subjek hanya menyebut topik konferensi")),
        "opening" => Some(t("No headline: these are the opening words of the text", "Tanpa kepala berita: ini adalah kata-kata pembuka teks")),
        "duplicate" => Some(t("Title from another copy of the same text", "Judul diambil dari salinan lain teks yang sama")),
        "topic" => Some(t("The subject line names the conference topic, not this message", "Baris subjek menyebut topik konferensi, bukan pesan ini")),
        _ => None,
    }
}

/// Snippets come back with \u{1}..\u{2} around matches; escape, then mark.
fn snippet_html(s: &str) -> Markup {
    let e = maud::html! { (s) }.into_string();
    PreEscaped(e.replace('\u{1}', "<mark>").replace('\u{2}', "</mark>"))
}

pub fn list(rows: &[Row], hl: Option<&str>, u: &User) -> Markup {
    html! {
        ul.results {
            @for r in rows {
                li {
                    div.meta {
                        @if let Some(d) = &r.date { a href=(format!("/read/day/{}", d)) { (nice_date(d)) } } @else { (t("undated", "tanpa tanggal")) }
                        @if let Some(c) = &r.code { " · " a href=(format!("/read/source/{}", urlencoding::encode(c))) title=[r.source_name.as_deref()] { (c) } }
                        " · " (lang_label(&r.lang)) " · " (type_label(&r.mtype)) " · " (fmt_n(r.words)) " " (t("words", "kata"))
                        @if r.dup { " · " span.chip { (t("duplicate copy", "salinan ganda")) } }
                        @if r.restricted { " · " span.chip title=(t("Restricted to researchers", "Hanya untuk peneliti")) { (t("restricted", "terbatas")) } }
                        " · " span.small { (r.ak_id) }
                    }
                    div.title {
                        a href=(match hl { Some(q) => format!("/read/m/{}?hl={}", r.ak_id, urlencoding::encode(q)), None => format!("/read/m/{}", r.ak_id) }) class=[if r.title_source.as_deref() == Some("opening") { Some("opening") } else { None }] title=[r.title_source.as_deref().and_then(title_note)] { (shown(u, &r.title)) }
                        @if let Some(tp) = &r.topic { " " a.chip.topic href=(format!("/read/topic?name={}", urlencoding::encode(tp))) title=(t("Conference topic this reply was filed under", "Topik konferensi tempat balasan ini diajukan")) { (tp) } }
                    }
                    @if let Some(s) = &r.snippet { div.snip { (snippet_html(&shown(u, s))) } }
                }
            }
        }
    }
}

fn pager(base: &str, page_n: i64, total: i64, per: i64) -> Markup {
    let pages = (total + per - 1) / per;
    let sep = if base.contains('?') { "&" } else { "?" };
    html! {
        @if pages > 1 {
            div.pager {
                @if page_n > 1 { a href=(format!("{base}{sep}page={}", page_n - 1)) { (t("← Previous", "← Sebelumnya")) } }
                span.small { (t("Page", "Halaman")) " " (page_n) " " (t("of", "dari")) " " (fmt_n(pages)) }
                @if page_n < pages { a href=(format!("{base}{sep}page={}", page_n + 1)) { (t("Next →", "Berikutnya →")) } }
            }
        }
    }
}

fn kicker_home() -> Markup {
    html! { a href="/read" { (t("Reading room", "Ruang baca")) } }
}

// ---------------------------------------------------------------- home

pub async fn home(State(st): State<AppState>, Reader(u): Reader) -> Result<Markup, AppError> {
    // everything on this page except the bookmark count is precomputed by `analyze`
    let uid = u.id;
    let bm: i64 = site(&st, move |c| c.bookmark_count(uid)).await?;
    let sv = |v: &serde_json::Value| v.as_str().unwrap_or("").to_string();
    let iv = |v: &serde_json::Value| v.as_i64().unwrap_or(0);
    let cal: std::collections::BTreeMap<String, i64> = st.a("monthly")["months"].as_array().cloned().unwrap_or_default().iter().filter(|m| iv(&m["total"]) > 0).map(|m| (sv(&m["ym"]), iv(&m["total"]))).collect();
    let mut themes: Vec<(String, String, i64)> = st.a("themes")["themes"].as_array().cloned().unwrap_or_default().iter().map(|tt| (sv(&tt["slug"]), i18n::theme_name(&sv(&tt["slug"]), &sv(&tt["name"])), iv(&tt["total"]))).collect();
    themes.sort_by(|a, b| b.2.cmp(&a.2));
    let sources: Vec<(String, Option<String>, i64)> = st.a("sources")["top"].as_array().cloned().unwrap_or_default().iter().take(40).map(|x| (sv(&x["code"]), x["name"].as_str().map(|y| y.to_string()), iv(&x["n"]))).collect();
    let ents: Vec<(String, String, String, i64)> = st.a("entities")["entities"].as_array().cloned().unwrap_or_default().iter().map(|e| (sv(&e["slug"]), sv(&e["name"]), sv(&e["kind"]), iv(&e["n"]))).collect();
    let max = cal.values().copied().max().unwrap_or(1) as f64;
    let who = if u.name.is_empty() { u.email.clone() } else { u.name.clone() };
    let body = html! {
        div.wide {
            div.kicker { (t("Reading room", "Ruang baca")) }
            h1 { (t("Every recovered message, 1990–2002", "Semua pesan yang berhasil dipulihkan, 1990–2002")) }
            p.lede { (tf(|| format!("Welcome, {who}. Search in Indonesian or English, browse by date, source, theme or name, and cite each message by its permanent ID."), || format!("Selamat datang, {who}. Cari dalam bahasa Indonesia atau Inggris, telusuri menurut tanggal, sumber, tema atau nama, dan kutip setiap pesan dengan ID permanennya."))) }
            @if !u.can_see_sensitive() { p.small { (t("Messages that may name survivors of sexual violence or private people are open only to approved researchers; until an editor has reviewed them they do not appear in lists or searches.", "Pesan yang mungkin menyebut nama penyintas kekerasan seksual atau orang biasa hanya terbuka bagi peneliti yang disetujui; selama belum ditinjau editor, pesan itu tidak muncul di daftar maupun hasil pencarian.")) } }
            (search_form(&SearchQ::default(), &[]))
            h2 { (t("Browse by month", "Telusuri menurut bulan")) }
            p.small { (t("Darker cells hold more messages. Duplicate copies are not counted. Click a month to see its days.", "Sel yang lebih gelap berisi lebih banyak pesan. Salinan ganda tidak dihitung. Klik sebuah bulan untuk melihat hari-harinya.")) }
            div.cal {
                span {}
                @for m in 1..=12 { span.small { (month_short(m)) } }
                @for y in 1990..=2002 {
                    span.y { (y) }
                    @for mo in 1..=12 {
                        @let k = format!("{y}-{mo:02}");
                        @match cal.get(&k) {
                            Some(n) => {
                                @let a = ((*n as f64 / max).sqrt() * 100.0).round();
                                a href=(format!("/read/month/{k}")) title=(format!("{}: {} {}", charts::nice_ym(&k), fmt_n(*n), t("messages", "pesan"))) style=(format!("background:color-mix(in srgb, var(--accent) {a}%, var(--chip));color:{}", if a > 60.0 { "var(--paper)" } else { "var(--ink)" })) { (if *n >= 1000 { format!("{}{}", i18n::dec(*n as f64 / 1000.0, 1), t("k", " rb")) } else { n.to_string() }) }
                            }
                            None => span style="background:var(--chip);opacity:.35" { "" }
                        }
                    }
                }
            }
            div.grid3 style="margin-top:34px" {
                div.card {
                    h3 { (t("Collections", "Koleksi")) }
                    p.small { (t("Curated entry points for close reading.", "Pintu masuk pilihan untuk pembacaan dekat.")) }
                    ul {
                        li { a href="/read/collection/may-1998" { (t("May 1998: riots and sexual violence", "Mei 1998: kerusuhan dan kekerasan seksual")) } " " span.small { "(" a href="/read/collection/may-1998?tier=3" { (t("focused only", "yang terfokus saja")) } ")" } }
                        li { a href="/read/collection/sexual-violence" { (t("Sexual violence (all years)", "Kekerasan seksual (semua tahun)")) } }
                        li { a href="/read/collection/gerwani" { (t("Gerwani and 1965", "Gerwani dan 1965")) } }
                        li { a href="/read/collection/macdougall" { (t("MacDougall's own notes and summaries", "Catatan dan ringkasan MacDougall sendiri")) } }
                        li { a href="/read/collection/statements" { (t("Statements, petitions and appeals", "Pernyataan, petisi dan seruan")) } }
                        li { a href="/read/collection/underground" { (t("Underground press: SiaR, Xpos, KdP", "Pers bawah tanah: SiaR, Xpos, KdP")) } }
                        li { a href="/read/collection/literary" { (t("Poems, humour, stories", "Puisi, humor, cerita")) } }
                        li { a href="/read/topics" { (t("Conference topics (1990–1999)", "Topik konferensi (1990–1999)")) } }
                        li { a href="/read/lost" { (t("Lost messages", "Pesan yang hilang")) } " " (t("(named in the database's listings, never captured)", "(disebut dalam daftar folder basis data, tidak pernah terekam)")) }
                        li { a href="/read/collection/iri-l" { (t("IRI-L (an unrelated list kept apart)", "IRI-L (milis lain yang dipisahkan)")) } }
                        li { a href="/read/bookmarks" { (t("Your bookmarks", "Penanda Anda")) " (" (bm) ")" } }
                    }
                    h3 { (t("Events", "Peristiwa")) }
                    ul { @for e in lexicon::EVENTS { li { a href=(format!("/read/event/{}", e.slug)) { (i18n::event_name(e.slug, e.name)) } " " span.small { (nice_date(e.date)) } } } }
                }
                div.card {
                    h3 { (t("Themes", "Tema")) }
                    @for (slug, name, n) in &themes { div { a href=(format!("/read/theme/{slug}")) { (name) } " " span.small { (fmt_n(*n)) } } }
                    h3 { (t("Sources", "Sumber")) }
                    @for (code, name, n) in &sources { a.chip href=(format!("/read/source/{}", urlencoding::encode(code))) title=[name.as_deref()] { (code) " " (fmt_n(*n)) } }
                }
                div.card {
                    h3 { (t("People", "Tokoh")) }
                    @for (slug, name, _, n) in ents.iter().filter(|e| e.2 == "person").take(40) { a.chip href=(format!("/read/entity/{slug}")) { (name) " " (fmt_n(*n)) } }
                    h3 { (t("Organisations", "Organisasi")) }
                    @for (slug, name, _, n) in ents.iter().filter(|e| e.2 == "org").take(30) { a.chip href=(format!("/read/entity/{slug}")) { (name) " " (fmt_n(*n)) } }
                    h3 { (t("Places", "Tempat")) }
                    @for (slug, name, _, n) in ents.iter().filter(|e| e.2 == "place") { a.chip href=(format!("/read/entity/{slug}")) { (name) " " (fmt_n(*n)) } }
                }
            }
        }
    };
    Ok(page(t("Reading room", "Ruang baca"), Some(&u), "read", body))
}

// ---------------------------------------------------------------- search

#[derive(Deserialize, Default, Clone)]
pub struct SearchQ {
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub theme: String,
    #[serde(default)]
    pub sort: String,
    #[serde(default)]
    pub dups: String,
    #[serde(default)]
    pub coll: String,
    #[serde(default)]
    pub page: Option<i64>,
}

fn search_form(q: &SearchQ, sources: &[(String, i64)]) -> Markup {
    html! {
        form action="/read/search" method="get" {
            div.searchbar {
                input type="search" name="q" value=(q.q) placeholder=(t("Search: perkosaan, \"Tim Relawan\", Gerwani -pki", "Cari: perkosaan, \"Tim Relawan\", Gerwani -pki")) autofocus;
                button.btn type="submit" { (t("Search", "Cari")) }
            }
            details open[!q.from.is_empty() || !q.to.is_empty() || !q.source.is_empty() || !q.lang.is_empty() || !q.r#type.is_empty() || !q.theme.is_empty()] {
                summary { (t("Filters", "Saringan")) }
                div.filters {
                    label { (t("From", "Dari")) " " input type="date" name="from" value=(q.from) min="1990-10-01" max="2002-03-01"; }
                    label { (t("to", "sampai")) " " input type="date" name="to" value=(q.to) min="1990-10-01" max="2002-03-01"; }
                    select name="lang" {
                        option value="" { (t("Any language", "Semua bahasa")) }
                        @for (v, l) in [("id", t("Indonesian", "Indonesia")), ("en", t("English", "Inggris")), ("mixed", t("Mixed", "Campuran"))] { option value=(v) selected[q.lang == v] { (l) } }
                    }
                    select name="type" {
                        option value="" { (t("Any type", "Semua jenis")) }
                        @for ty in ["news", "reply", "opinion", "statement", "rubric", "literary", "other"] { option value=(ty) selected[q.r#type == ty] { (type_label(ty)) } }
                    }
                    select name="theme" {
                        option value="" { (t("Any theme", "Semua tema")) }
                        @for th in lexicon::THEMES { option value=(th.slug) selected[q.theme == th.slug] { (i18n::theme_name(th.slug, th.name)) } }
                    }
                    input type="text" name="source" value=(q.source) placeholder=(t("Source code (KMP, JKTP, SIAR…)", "Kode sumber (KMP, JKTP, SIAR…)")) style="width:200px" list="srcs";
                    datalist id="srcs" { @for (c, _) in sources { option value=(c) {} } }
                    select name="sort" {
                        option value="" { (t("Best match", "Paling cocok")) }
                        option value="date" selected[q.sort == "date"] { (t("Oldest first", "Terlama dahulu")) }
                        option value="date_desc" selected[q.sort == "date_desc"] { (t("Newest first", "Terbaru dahulu")) }
                    }
                    select name="coll" {
                        option value="" { (t("apakabar (database and list archives)", "apakabar (basis data dan arsip milis)")) }
                        option value="basisdata" selected[q.coll == "basisdata"] { (t("database only", "basis data saja")) }
                        option value="list" selected[q.coll == "list"] { (t("list archives only", "arsip milis saja")) }
                        option value="iri-l" selected[q.coll == "iri-l"] { (t("IRI-L (unrelated list)", "IRI-L (milis lain)")) }
                    }
                    label { input type="checkbox" name="dups" value="1" checked[q.dups == "1"]; " " (t("include duplicate copies", "sertakan salinan ganda")) }
                }
            }
        }
    }
}

static WORD: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(-?)"([^"]+)"|(-?)([\p{L}\p{N}][\p{L}\p{N}'-]*)"#).unwrap());

/// Turn what a reader types into an FTS5 expression. Words are AND-ed; each word also
/// matches its Indonesian stem ("perkosaan" finds "diperkosa"); "quoted phrases" stay
/// phrases; a leading minus excludes; OR between two words is kept.
pub fn fts_query(input: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut pending_or = false;
    for m in WORD.captures_iter(input) {
        if let Some(ph) = m.get(2) {
            let neg = !m.get(1).map(|x| x.as_str()).unwrap_or("").is_empty();
            let p = format!("\"{}\"", ph.as_str().replace('"', ""));
            push(&mut parts, if neg { format!("NOT {p}") } else { p }, &mut pending_or);
        } else if let Some(w) = m.get(4) {
            let w = w.as_str();
            if w == "OR" {
                pending_or = true;
                continue;
            }
            if w == "AND" {
                continue;
            }
            let neg = !m.get(3).map(|x| x.as_str()).unwrap_or("").is_empty();
            let lw = w.to_lowercase().replace('\'', "");
            if lw.is_empty() {
                continue;
            }
            let st = text::stem(&lw);
            let term = if lw.chars().count() >= 3 { format!("\"{}\"*", lw) } else { format!("\"{}\"", lw) };
            let expr = if st != lw && st.chars().count() >= 3 { format!("({term} OR stems:\"{st}\")") } else { term };
            push(&mut parts, if neg { format!("NOT {expr}") } else { expr }, &mut pending_or);
        }
    }
    fn push(parts: &mut Vec<String>, e: String, pending_or: &mut bool) {
        if *pending_or && !parts.is_empty() && !e.starts_with("NOT") {
            let last = parts.pop().unwrap();
            parts.push(format!("({last} OR {e})"));
        } else {
            parts.push(e);
        }
        *pending_or = false;
    }
    // NOT cannot start an FTS5 query
    let pos: Vec<&String> = parts.iter().filter(|p| !p.starts_with("NOT")).collect();
    if pos.is_empty() {
        return None;
    }
    let mut out = pos.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" AND ");
    for n in parts.iter().filter(|p| p.starts_with("NOT")) {
        out = format!("{out} {n}");
    }
    Some(out)
}

pub async fn search(State(st): State<AppState>, Reader(u): Reader, Query(q): Query<SearchQ>) -> Result<Markup, AppError> {
    let per = 30i64;
    let page_n = q.page.unwrap_or(1).max(1);
    let fq = fts_query(&q.q);
    let q2 = q.clone();
    let vis = visible(&u);
    let res = blocking(&st, move |c| {
        let mut wh: Vec<String> = vec![vis.to_string()];
        let mut args: Vec<rusqlite::types::Value> = Vec::new();
        if q2.dups != "1" { wh.push("m.dup_of IS NULL".into()); }
        match q2.coll.as_str() {
            "basisdata" | "list" | "iri-l" => { wh.push("m.collection = ?".into()); args.push(q2.coll.clone().into()); }
            _ => wh.push("m.collection != 'iri-l'".into()),
        }
        if !q2.from.is_empty() { wh.push("m.date >= ?".into()); args.push(q2.from.clone().into()); }
        if !q2.to.is_empty() { wh.push("m.date <= ?".into()); args.push(q2.to.clone().into()); }
        if !q2.lang.is_empty() { wh.push("m.lang = ?".into()); args.push(q2.lang.clone().into()); }
        if !q2.r#type.is_empty() { wh.push("m.msg_type = ?".into()); args.push(q2.r#type.clone().into()); }
        if !q2.source.is_empty() { wh.push("m.source_code = ?".into()); args.push(q2.source.trim().to_uppercase().into()); }
        if !q2.theme.is_empty() { wh.push("m.id IN (SELECT mt.msg_id FROM message_themes mt JOIN themes t ON t.id = mt.theme_id WHERE t.slug = ? AND mt.about = 1)".into()); args.push(q2.theme.clone().into()); }
        let order = match q2.sort.as_str() { "date" => "m.date IS NULL, m.date, m.id", "date_desc" => "m.date DESC, m.id DESC", _ => if fq.is_some() { "bm25(fts, 6.0, 1.0, 0.6)" } else { "m.date IS NULL, m.date, m.id" } };
        let (from, mut all_args) = match &fq {
            Some(f) => ("fts JOIN messages m ON m.id = fts.rowid".to_string(), vec![rusqlite::types::Value::from(f.clone())]),
            None => ("messages m".to_string(), vec![]),
        };
        let mut conds = wh.clone();
        if fq.is_some() { conds.insert(0, "fts MATCH ?".into()); }
        all_args.extend(args.iter().cloned());
        let where_sql = if conds.is_empty() { String::new() } else { format!("WHERE {}", conds.join(" AND ")) };
        let filtered = !q2.from.is_empty() || !q2.to.is_empty() || !q2.lang.is_empty() || !q2.r#type.is_empty() || !q2.source.is_empty() || !q2.theme.is_empty();
        if fq.is_none() && !filtered {
            return Ok((vec![], 0, vec![], None));
        }
        let snip = if fq.is_some() { ", snippet(fts, 1, char(1), char(2), ' … ', 28)" } else { ", NULL" };
        let sql = format!("SELECT {ROW_COLS}{snip} FROM {from} {where_sql} ORDER BY {order} LIMIT {per} OFFSET {}", (page_n - 1) * per);
        let rows: Vec<Row> = match c.prepare(&sql) {
            Ok(mut s) => s.query_map(params_from_iter(all_args.iter()), |r| row(r, true))?.collect::<rusqlite::Result<_>>()?,
            Err(e) => return Ok((vec![], 0, vec![], Some(e.to_string()))),
        };
        let total: i64 = c.query_row(&format!("SELECT count(*) FROM (SELECT 1 FROM {from} {where_sql} LIMIT 20001)"), params_from_iter(all_args.iter()), |r| r.get(0)).unwrap_or(0);
        let mut years: Vec<(i64, i64)> = Vec::new();
        if total > 0 {
            let mut s = c.prepare(&format!("SELECT m.year, count(*) FROM {from} {where_sql} GROUP BY m.year"))?;
            for r in s.query_map(params_from_iter(all_args.iter()), |r| Ok((r.get::<_, Option<i64>>(0)?.unwrap_or(0), r.get::<_, i64>(1)?)))? {
                years.push(r?);
            }
        }
        Ok((rows, total, years, None))
    })
    .await?;
    let (rows, total, years, err) = res;
    let srcs = vec![];
    let base = format!(
        "/read/search?q={}&from={}&to={}&source={}&lang={}&type={}&theme={}&sort={}&dups={}&coll={}",
        urlencoding::encode(&q.q), q.from, q.to, urlencoding::encode(&q.source), q.lang, q.r#type, q.theme, q.sort, q.dups, q.coll
    );
    let ymax = years.iter().map(|y| y.1).max().unwrap_or(1).max(1);
    let body = html! {
        div.wide {
            div.kicker { (kicker_home()) " / " (t("search", "pencarian")) }
            (search_form(&q, &srcs))
            @if let Some(e) = err { div.flash { (t("That search could not be read", "Pencarian itu tidak dapat dibaca")) " (" (e) "). " (t("Try plain words, \"phrases in quotes\", OR between alternatives and -word to exclude.", "Gunakan kata biasa, \"frasa dalam tanda kutip\", OR di antara pilihan, dan -kata untuk mengecualikan.")) } }
            @if q.q.trim().is_empty() && rows.is_empty() { p.small { (t("Type a word or phrase. Indonesian words also match their other forms: ", "Ketik sebuah kata atau frasa. Kata bahasa Indonesia juga cocok dengan bentuk lainnya: ")) em { "perkosaan" } (t(" finds ", " menemukan ")) em { "diperkosa" } (t(" and ", " dan ")) em { "pemerkosaan" } "." } }
            @else {
                p { b { (if total > 20000 { t("More than 20,000", "Lebih dari 20.000").to_string() } else { fmt_n(total) }) } " " (t("messages", "pesan")) @if q.dups != "1" { " " (t("(duplicate copies hidden)", "(salinan ganda disembunyikan)")) } "." }
                @if !years.is_empty() {
                    div.figure.inline {
                        h4 { (t("Results by year", "Hasil per tahun")) }
                        div style="display:grid;grid-template-columns:repeat(13,1fr);gap:4px;align-items:end;height:80px" {
                            @for y in 1990..=2002 {
                                @let n = years.iter().find(|x| x.0 == y).map(|x| x.1).unwrap_or(0);
                                a href=(format!("{base}&from={y}-01-01&to={y}-12-31")) title=(format!("{y}: {n}")) style=(format!("display:block;height:{}%;background:var(--accent);min-height:{}px;border-radius:2px", n * 100 / ymax, if n > 0 { 2 } else { 0 })) {}
                            }
                        }
                        div style="display:grid;grid-template-columns:repeat(13,1fr);gap:4px;font:11px var(--sans);color:var(--ink3);text-align:center" { @for y in 1990..=2002 { span { (format!("'{:02}", y % 100)) } } }
                    }
                }
                (list(&rows, Some(&q.q), &u))
                (pager(&base, page_n, total.min(20000), per))
            }
        }
    };
    Ok(page(t("Search", "Pencarian"), Some(&u), "read", body))
}

// ---------------------------------------------------------------- one message

#[derive(Deserialize)]
pub struct HlQ {
    #[serde(default)]
    pub hl: String,
}

struct Msg {
    id: i64,
    ak_id: String,
    collection: String,
    list_tag: Option<String>,
    date: Option<String>,
    subject: String,
    title: String,
    sender: Option<String>,
    orig_from: Option<String>,
    orig_date: Option<String>,
    orig_subject: Option<String>,
    code: Option<String>,
    source_basis: Option<String>,
    source_name: Option<String>,
    source_cat: Option<String>,
    lang: String,
    mtype: String,
    topic_reply: bool,
    title_source: Option<String>,
    topic: Option<String>,
    date_note: Option<String>,
    may_tier: i64,
    dup_of: Option<i64>,
    thread_id: Option<String>,
    words: i64,
    headers: Option<String>,
    body: String,
    orig_url: Option<String>,
    wayback: Option<String>,
    repaired: i64,
    sensitive: bool,
}

fn load_msg(c: &rusqlite::Connection, ak_id: &str) -> rusqlite::Result<Option<Msg>> {
    c.query_row(
        "SELECT id, ak_id, collection, list_tag, date, subject, title, sender, orig_from, orig_date, orig_subject, source_code, source_basis, source_name, source_cat,
                lang, msg_type, is_topic_reply, dup_of, thread_id, words, headers, body, orig_url, wayback_url, repaired, sensitive, title_source, topic, date_note, may98_tier FROM messages WHERE ak_id = ?1",
        [ak_id],
        |r| {
            Ok(Msg {
                id: r.get(0)?, ak_id: r.get(1)?, collection: r.get(2)?, list_tag: r.get(3)?, date: r.get(4)?, subject: r.get(5)?, title: r.get(6)?,
                sender: r.get(7)?, orig_from: r.get(8)?, orig_date: r.get(9)?, orig_subject: r.get(10)?, code: r.get(11)?, source_basis: r.get(12)?,
                source_name: r.get(13)?, source_cat: r.get(14)?, lang: r.get(15)?, mtype: r.get(16)?, topic_reply: r.get::<_, i64>(17)? == 1, dup_of: r.get(18)?,
                thread_id: r.get(19)?, words: r.get(20)?, headers: r.get(21)?, body: r.get(22)?, orig_url: r.get(23)?, wayback: r.get(24)?, repaired: r.get(25)?,
                sensitive: r.get::<_, i64>(26)? == 1,
                title_source: r.get(27)?, topic: r.get(28)?, date_note: r.get(29)?, may_tier: r.get(30)?,
            })
        },
    )
    .optional()
}

fn render_body(body: &str, hl: &str) -> Markup {
    let terms: Vec<String> = WORD
        .captures_iter(hl)
        .filter_map(|m| m.get(2).or(m.get(4)).map(|x| x.as_str().to_string()))
        .filter(|w| w.chars().count() >= 3 && w != "AND" && w != "OR")
        .collect();
    let re = if terms.is_empty() {
        None
    } else {
        let alts: Vec<String> = terms.iter().map(|tm| {
            let st = text::stem(&tm.to_lowercase());
            if st != tm.to_lowercase() && st.chars().count() >= 3 { format!("{}|\\w*{}\\w*", regex::escape(tm), regex::escape(&st)) } else { format!("{}\\w*", regex::escape(tm)) }
        }).collect();
        Regex::new(&format!("(?i)\\b(?:{})", alts.join("|"))).ok()
    };
    let mark = |s: &str| -> PreEscaped<String> {
        let e = html! { (s) }.into_string();
        match &re {
            Some(re) => PreEscaped(re.replace_all(&e, "<mark>$0</mark>").into_owned()),
            None => PreEscaped(e),
        }
    };
    html! {
        @for p in body.split("\n\n") {
            @if let Some(q) = p.strip_prefix("> ") { blockquote { @for (i, l) in q.split('\n').enumerate() { @if i > 0 { br; } (mark(l)) } } }
            @else if p.contains('\n') && p.lines().any(|l| l.trim_start() != l || l.contains("   ")) { pre { (mark(p)) } }
            @else if p.contains('\n') { p { @for (i, l) in p.split('\n').enumerate() { @if i > 0 { br; } (mark(l)) } } }
            @else { p { (mark(p)) } }
        }
    }
}

/// APA and Chicago citations (English, as both styles are).
fn citation(m: &Msg) -> (String, String) {
    let author = m.orig_from.as_deref().filter(|s| !s.is_empty()).map(|s| s.split('<').next().unwrap_or(s).trim().trim_matches('"').to_string()).filter(|s| !s.is_empty() && !s.contains('@')).unwrap_or_else(|| "apakabar".into());
    let (y, md) = match &m.date {
        Some(d) => {
            let mo: usize = d[5..7].parse().unwrap_or(1);
            (d[..4].to_string(), format!("{} {}", i18n::MONTHS_LONG_EN[mo.clamp(1, 12) - 1], d[8..10].trim_start_matches('0')))
        }
        None => ("n.d.".into(), String::new()),
    };
    let en_date = m.date.as_deref().map(|d| {
        let mo: usize = d[5..7].parse().unwrap_or(1);
        format!("{} {} {}", d[8..10].trim_start_matches('0'), i18n::MONTHS_LONG_EN[mo.clamp(1, 12) - 1], &d[..4])
    });
    let url = m.wayback.clone().unwrap_or_default();
    let apa = format!("{author}. ({y}{}). {} [Mailing list message, {}]. apakabar (Indonesia Publications, ed. J. A. MacDougall). {url}", if md.is_empty() { String::new() } else { format!(", {md}") }, m.title, m.ak_id);
    let chicago = format!("{}\u{201C}{},\u{201D} apakabar mailing list (ed. John A. MacDougall), {}, {}, {url}.", if author == "apakabar" { String::new() } else { format!("{author}, ") }, m.title, en_date.unwrap_or_else(|| "n.d.".into()), m.ak_id);
    (apa, chicago)
}

fn limit_page(u: &User) -> Markup {
    page(t("Daily limit", "Batas harian"), Some(u), "read", html! { div.essay {
        h1 { (t("Daily reading limit reached", "Batas baca harian tercapai")) }
        p { (tf(|| format!("To protect the archive from bulk copying, each account can open {DAILY_LIMIT} messages a day. Please continue tomorrow, or write to the editors if your research needs more."), || format!("Untuk melindungi arsip dari penyalinan massal, setiap akun dapat membuka {DAILY_LIMIT} pesan per hari. Silakan lanjutkan besok, atau hubungi editor bila penelitian Anda memerlukan lebih."))) }
    } })
}

pub async fn message(State(st): State<AppState>, Reader(u): Reader, Path(ak_id): Path<String>, Query(hq): Query<HlQ>) -> Result<Response, AppError> {
    let uid = u.id;
    let n = site(&st, move |c| c.count_read(uid)).await?;
    if n > DAILY_LIMIT {
        return Ok((StatusCode::TOO_MANY_REQUESTS, limit_page(&u)).into_response());
    }
    let vis = visible(&u);
    let ak2 = ak_id.clone();
    let data = blocking(&st, move |c| {
        let m = match load_msg(c, &ak2)? { Some(m) => m, None => return Ok(None) };
        let themes: Vec<(String, String, bool)> = c.prepare("SELECT t.slug, t.name, mt.about FROM message_themes mt JOIN themes t ON t.id = mt.theme_id WHERE mt.msg_id = ?1 ORDER BY mt.about DESC, mt.score DESC")?
            .query_map([m.id], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? == 1)))?.collect::<rusqlite::Result<_>>()?;
        let ents: Vec<(String, String, String)> = c.prepare("SELECT e.slug, e.name, e.kind FROM message_entities me JOIN entities e ON e.id = me.entity_id WHERE me.msg_id = ?1 ORDER BY me.hits DESC LIMIT 30")?
            .query_map([m.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        let root = m.dup_of.unwrap_or(m.id);
        let copies: Vec<Row> = c.prepare(&format!("SELECT {ROW_COLS} FROM messages m WHERE (m.id = ?1 OR m.dup_of = ?1) AND m.id != ?2 AND {vis} ORDER BY m.id"))?
            .query_map(params![root, m.id], |r| row(r, false))?.collect::<rusqlite::Result<_>>()?;
        let thread: Vec<Row> = match &m.thread_id {
            Some(tid) => c.prepare(&format!("SELECT {ROW_COLS} FROM messages m WHERE m.thread_id = ?1 AND {vis} ORDER BY m.date, m.id LIMIT 60"))?.query_map([tid], |r| row(r, false))?.collect::<rusqlite::Result<_>>()?,
            None => vec![],
        };
        let topic_n: i64 = match &m.topic {
            Some(tp) => c.query_row("SELECT count(*) FROM messages WHERE topic = ?1 AND counted = 1", [tp], |r| r.get(0))?,
            None => 0,
        };
        let prev: Option<(String, String)> = c.query_row(&format!("SELECT m.ak_id, m.title FROM messages m WHERE m.id < ?1 AND m.collection = ?2 AND {vis} ORDER BY m.id DESC LIMIT 1"), params![m.id, m.collection], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        let next: Option<(String, String)> = c.query_row(&format!("SELECT m.ak_id, m.title FROM messages m WHERE m.id > ?1 AND m.collection = ?2 AND {vis} ORDER BY m.id LIMIT 1"), params![m.id, m.collection], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        Ok(Some((m, themes, ents, copies, thread, prev, next, topic_n)))
    })
    .await?;
    let (m, themes, ents, copies, thread, prev, next, topic_n) = match data {
        None => return Ok((StatusCode::NOT_FOUND, page(t("Not found", "Tidak ditemukan"), Some(&u), "read", html! { div.essay { h1 { (t("No message with that ID", "Tidak ada pesan dengan ID itu")) } p { a href="/read" { (t("Back to the reading room", "Kembali ke ruang baca")) } } } })).into_response()),
        Some(x) => x,
    };
    let (uid, ak3) = (u.id, m.ak_id.clone());
    let bm = site(&st, move |c| c.bookmark_note(uid, &ak3)).await?;
    let hidden = m.sensitive && !u.can_see_sensitive();
    if hidden {
        // readers learn only that the message exists, not what it says or whom it names
        let body = html! {
            div.kicker.noprint { (kicker_home()) }
            div.essay {
                div.small { (m.ak_id) @if let Some(d) = &m.date { " · " (nice_date(d)) } }
                h1 { (t("Restricted message", "Pesan terbatas")) }
                div.note { (t("This message may name survivors of sexual violence or private people. It is open to approved researchers; until an editor has reviewed it, its title and text are withheld. Write to the editors if your research needs it.", "Pesan ini mungkin menyebut nama penyintas kekerasan seksual atau orang biasa. Pesan ini terbuka bagi peneliti yang disetujui; selama belum ditinjau editor, judul dan isinya ditahan. Hubungi editor bila penelitian Anda memerlukannya.")) }
                p { a href="/read" { (t("Back to the reading room", "Kembali ke ruang baca")) } }
            }
        };
        return Ok(page(t("Restricted message", "Pesan terbatas"), Some(&u), "read", body).into_response());
    }
    let (apa, chicago) = citation(&m);
    let (apa, chicago) = (shown(&u, &apa), shown(&u, &chicago));
    let tier_label = match m.may_tier {
        3 => t("focused on the sexual violence of May 1998", "berfokus pada kekerasan seksual Mei 1998"),
        2 => t("mentions the sexual violence with the May riots", "menyebut kekerasan seksual bersama kerusuhan Mei"),
        _ => t("mentions sexual violence", "menyebut kekerasan seksual"),
    };
    let collection = match m.collection.as_str() {
        "basisdata" => t("apakabar database (Hamline / Ohio)", "basis data apakabar (Hamline / Ohio)"),
        "iri-l" => t("IRI-L, an unrelated list in the indopubs.com archives", "IRI-L, milis lain dalam arsip indopubs.com"),
        _ => t("indopubs.com list archive", "arsip milis indopubs.com"),
    };
    let body = html! {
        div.kicker.noprint { (kicker_home()) @if let Some(d) = &m.date { " / " a href=(format!("/read/month/{}", &d[..7])) { (charts::nice_ym(&d[..7])) } " / " a href=(format!("/read/day/{d}")) { (d[8..10].trim_start_matches('0')) } } }
        div.reader {
            article.msg {
                div.small { (m.ak_id) @if let Some(tg) = &m.list_tag { " · " (tg) } @if m.topic_reply { " · " (t("reply under a conference topic", "balasan di bawah topik konferensi")) } @if m.sensitive { " · " span.chip { (t("restricted to researchers", "hanya untuk peneliti")) } } }
                h1 { (m.title) }
                @if let Some(n) = m.title_source.as_deref().and_then(title_note) { p.small.titlenote { (n) ". " (t("Subject as posted", "Subjek asli")) ": \u{201c}" (m.subject) "\u{201d}" } }
                div.byline {
                    @if let Some(d) = &m.date { (nice_date(d)) }
                    @if let Some(n) = &m.source_name { " · " (n) } @else if let Some(c) = &m.code { " · " (c) }
                    @if let Some(f) = m.orig_from.as_deref().filter(|s| !s.is_empty()) { " · " (t("from", "dari")) " " (shown(&u, f)) }
                }
                @if let Some(dn) = &m.date_note { p.small { (t("Date", "Tanggal")) ": " (dn) "." } }
                @if m.repaired > 0 { p.small { (tf(|| format!("Characters lost in capture were restored ({}): apostrophes and quotation marks only.", m.repaired), || format!("Karakter yang hilang saat perekaman dipulihkan ({}): hanya apostrof dan tanda kutip.", m.repaired))) } }
                div.body { (render_body(&shown(&u, &m.body), &hq.hl)) }
                @if let Some(h) = m.headers.as_deref().filter(|h| !h.is_empty()) {
                    details { summary { (t("Mail headers inside the message", "Kepala surat di dalam pesan")) } pre.raw { (shown(&u, h)) } }
                }
                div.pager.noprint {
                    @if let Some((id, tt)) = &prev { a href=(format!("/read/m/{id}")) title=(tt) { (t("← Previous message", "← Pesan sebelumnya")) } }
                    @if let Some((id, tt)) = &next { a href=(format!("/read/m/{id}")) title=(tt) { (t("Next message →", "Pesan berikutnya →")) } }
                }
            }
            aside.side {
                h4 { (t("Cite", "Kutip")) }
                div.cite { (apa) }
                details { summary { (t("Chicago style", "Gaya Chicago")) } div.cite { (chicago) } }
                h4 { (t("Record", "Catatan")) }
                dl {
                    dt { (t("Date", "Tanggal")) } dd { (m.date.as_deref().map(nice_date).unwrap_or_else(|| t("undated", "tanpa tanggal").into())) }
                    dt { (t("Source", "Sumber")) } dd {
                        @match &m.code {
                            Some(c) => { a href=(format!("/read/source/{}", urlencoding::encode(c))) { (m.source_name.clone().unwrap_or_else(|| c.clone())) } br; span.small { (i18n::category_label(m.source_cat.as_deref().unwrap_or(""))) @if let Some(b) = &m.source_basis { ", " (t("read from the", "dibaca dari")) " " (b) } } }
                            None => (t("not identified", "tidak diketahui")),
                        }
                    }
                    dt { (t("Language", "Bahasa")) } dd { (lang_label(&m.lang)) }
                    dt { (t("Type", "Jenis")) } dd { (type_label(&m.mtype)) ", " (fmt_n(m.words)) " " (t("words", "kata")) }
                    dt { (t("Collection", "Koleksi")) } dd { (collection) }
                    @if let Some(tp) = &m.topic { dt { (t("Conference topic", "Topik konferensi")) } dd { a href=(format!("/read/topic?name={}", urlencoding::encode(tp))) { (tp) } " " span.small { "(" (fmt_n(topic_n)) " " (t("messages", "pesan")) ")" } } }
                    @if m.may_tier > 0 { dt { (t("May 1998 flag", "Penanda Mei 1998")) } dd { (tier_label) } }
                    @if let Some(s) = m.orig_subject.as_deref().filter(|s| !s.is_empty()) { dt { (t("Original subject", "Subjek asli di dalam pesan")) } dd { (s) } }
                    @if let Some(s) = m.orig_date.as_deref().filter(|s| !s.is_empty()) { dt { (t("Original date line", "Baris tanggal asli")) } dd { (s) } }
                    dt { (t("Posted by", "Dikirim oleh")) } dd { (shown(&u, &m.sender.clone().unwrap_or_default())) }
                    dt { (t("Subject as posted", "Subjek seperti dikirim")) } dd { (m.subject) }
                }
                h4 { (t("Provenance", "Asal-usul")) }
                p {
                    @if let Some(w) = &m.wayback { a href=(w) rel="noopener" { (t("Internet Archive capture", "Rekaman Internet Archive")) } br; }
                    @if let Some(o) = &m.orig_url { span.small { (o) } br; }
                    a href=(format!("/read/m/{}/raw", m.ak_id)) { (t("Raw text as recovered", "Teks mentah seperti dipulihkan")) }
                }
                @if !themes.is_empty() { h4 { (t("Themes", "Tema")) } @for (s, n, about) in &themes { @if *about { a.chip href=(format!("/read/theme/{s}")) title=(t("The message is about this theme", "Pesan ini membahas tema ini")) { (i18n::theme_name(s, n)) } } @else { a.chip.weak href=(format!("/read/theme/{s}?all=1")) title=(t("The message mentions this theme", "Pesan ini menyebut tema ini")) { (i18n::theme_name(s, n)) } } } p.small { (t("Solid: about the theme. Outlined: mentions it.", "Penuh: membahas tema. Bergaris: menyebutnya.")) } }
                @if !ents.is_empty() { h4 { (t("Names", "Nama")) } @for (s, n, _) in &ents { a.chip href=(format!("/read/entity/{s}")) { (n) } } }
                @if !copies.is_empty() {
                    h4 { (t("Other copies", "Salinan lain")) }
                    @for r in &copies { div { a href=(format!("/read/m/{}", r.ak_id)) { (r.ak_id) } " " span.small { (r.date.clone().unwrap_or_default()) } } }
                }
                @if !thread.is_empty() {
                    h4 { a href=(format!("/read/thread/{}", m.thread_id.clone().unwrap_or_default())) { (t("Thread", "Utas")) " (" (thread.len()) ")" } }
                    @for r in &thread { div { @if r.ak_id == m.ak_id { b { (r.title) } } @else { a href=(format!("/read/m/{}", r.ak_id)) { (r.title) } } " " span.small { (r.date.clone().unwrap_or_default()) } } }
                }
                h4 { (t("Your notes", "Catatan Anda")) }
                form method="post" action=(format!("/read/m/{}/bookmark", m.ak_id)) {
                    (csrf_field())
                    textarea name="note" placeholder=(t("Private note, saved with a bookmark", "Catatan pribadi, disimpan bersama penanda")) { (bm.clone().unwrap_or_default()) }
                    div style="display:flex;gap:6px;margin-top:6px" {
                        button.btn type="submit" { (if bm.is_some() { t("Save note", "Simpan catatan") } else { t("Bookmark", "Tandai") }) }
                        @if bm.is_some() { button.btn.ghost type="submit" name="remove" value="1" { (t("Remove", "Hapus")) } }
                    }
                }
                @if u.is_admin() {
                    h4 { "Editor" }
                    form method="post" action=(format!("/admin/sensitive/{}", m.ak_id)) { (csrf_field()) button.btn.ghost type="submit" { (if m.sensitive { "Unmark sensitive" } else { "Mark sensitive" }) } }
                    @if m.may_tier >= 2 { p.small { a href=(format!("/admin/review#{}", m.ak_id)) { "Open in the review page" } } }
                }
            }
        }
    };
    Ok(page(&m.title, Some(&u), "read", body).into_response())
}

pub async fn raw(State(st): State<AppState>, Reader(u): Reader, Path(ak_id): Path<String>) -> Result<Response, AppError> {
    let uid = u.id;
    let can = u.can_see_sensitive();
    let n = site(&st, move |c| c.count_read(uid)).await?;
    if n > DAILY_LIMIT {
        return Ok((StatusCode::TOO_MANY_REQUESTS, limit_page(&u)).into_response());
    }
    let r = blocking(&st, move |c| c.query_row("SELECT raw_z, sensitive FROM messages WHERE ak_id = ?1", [ak_id], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?))).optional()).await?;
    match r {
        Some((z, s)) if s == 0 || can => {
            let txt = zstd::decode_all(&z[..]).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
            Ok(([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], shown(&u, &txt)).into_response())
        }
        Some(_) => Ok((StatusCode::FORBIDDEN, t("Restricted message", "Pesan terbatas").to_string()).into_response()),
        None => Ok((StatusCode::NOT_FOUND, t("Not found", "Tidak ditemukan").to_string()).into_response()),
    }
}

#[derive(Deserialize)]
pub struct BmForm {
    #[serde(default)]
    note: String,
    #[serde(default)]
    remove: String,
}

pub async fn bookmark(State(st): State<AppState>, Reader(u): Reader, Path(ak_id): Path<String>, Form(f): Form<BmForm>) -> Result<Redirect, AppError> {
    let id2 = ak_id.clone();
    let known: Option<i64> = blocking(&st, move |c| c.query_row("SELECT id FROM messages WHERE ak_id = ?1", [&id2], |r| r.get(0)).optional()).await?;
    if known.is_some() {
        let (uid, id3) = (u.id, ak_id.clone());
        site(&st, move |c| {
            if f.remove == "1" {
                c.delete_bookmark(uid, &id3)
            } else {
                c.save_bookmark(uid, &id3, &f.note.chars().take(5000).collect::<String>())
            }
        })
        .await?;
    }
    Ok(Redirect::to(&format!("/read/m/{ak_id}")))
}

#[derive(Deserialize)]
pub struct FmtQ {
    #[serde(default)]
    format: String,
}

pub async fn bookmarks(State(st): State<AppState>, Reader(u): Reader, Query(fq): Query<FmtQ>) -> Result<Response, AppError> {
    let uid = u.id;
    let marks = site(&st, move |c| c.bookmarks(uid)).await?;
    let can = u.can_see_sensitive();
    let ids: Vec<String> = marks.iter().map(|m| m.0.clone()).collect();
    let meta: std::collections::HashMap<String, (Option<String>, String, Option<String>, Option<String>, bool)> = blocking(&st, move |c| {
        let mut out = std::collections::HashMap::new();
        let mut q = c.prepare("SELECT date, title, source_code, wayback_url, sensitive FROM messages WHERE ak_id = ?1")?;
        for a in &ids {
            if let Some(r) = q.query_row([a], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, i64>(4)? == 1))).optional()? {
                out.insert(a.clone(), r);
            }
        }
        Ok(out)
    })
    .await?;
    let missing = t("(not in the current database)", "(tidak ada di basis data saat ini)");
    let withheld = t("(restricted message)", "(pesan terbatas)");
    let mut rows: Vec<(String, Option<String>, String, Option<String>, String, Option<String>)> = marks
        .into_iter()
        .map(|(ak, note)| match meta.get(&ak) {
            Some((d, title, src, wb, sens)) => {
                let title = if *sens && !can { withheld.to_string() } else { title.clone() };
                (ak, d.clone(), title, src.clone(), note, wb.clone())
            }
            None => (ak, None, missing.to_string(), None, note, None),
        })
        .collect();
    rows.sort_by(|a, b| (a.1.clone(), a.0.clone()).cmp(&(b.1.clone(), b.0.clone())));
    if fq.format == "csv" {
        let mut out = String::from("ak_id,date,title,source,note,wayback_url\n");
        let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
        for r in &rows {
            out.push_str(&format!("{},{},{},{},{},{}\n", r.0, r.1.clone().unwrap_or_default(), q(&r.2), r.3.clone().unwrap_or_default(), q(&r.4), r.5.clone().unwrap_or_default()));
        }
        return Ok(([(header::CONTENT_TYPE, "text/csv; charset=utf-8"), (header::CONTENT_DISPOSITION, "attachment; filename=\"apakabar-bookmarks.csv\"")], out).into_response());
    }
    let body = html! {
        div.wide {
            div.kicker { (kicker_home()) " / " (t("bookmarks", "penanda")) }
            h1 { (t("Your bookmarks", "Penanda Anda")) }
            p { (rows.len()) " " (t("messages.", "pesan.")) " " @if !rows.is_empty() { a href="/read/bookmarks?format=csv" { (t("Download as CSV", "Unduh sebagai CSV")) } " " (t("(IDs, dates, titles, your notes and the archive links; no full texts).", "(ID, tanggal, judul, catatan Anda dan tautan arsip; tanpa teks lengkap).")) } }
            table.t {
                thead { tr { th { (t("Date", "Tanggal")) } th { (t("Message", "Pesan")) } th { (t("Source", "Sumber")) } th { (t("Your note", "Catatan Anda")) } } }
                tbody { @for r in &rows { tr { td { (r.1.clone().unwrap_or_default()) } td { a href=(format!("/read/m/{}", r.0)) { (r.2) } } td { (r.3.clone().unwrap_or_default()) } td { (r.4) } } } }
            }
        }
    };
    Ok(page(t("Bookmarks", "Penanda"), Some(&u), "read", body).into_response())
}

// ---------------------------------------------------------------- browse pages

#[derive(Deserialize)]
pub struct PageQ {
    pub page: Option<i64>,
}

#[allow(clippy::too_many_arguments)]
async fn listing(st: &AppState, u: &User, title: String, kicker: Markup, intro: Markup, where_sql: String, args: Vec<rusqlite::types::Value>, order: &'static str, page_n: i64, base: String) -> Result<Markup, AppError> {
    let per = 50i64;
    let w2 = format!("({where_sql}) AND {}", visible(u));
    let a2 = args.clone();
    let (rows, total) = blocking(st, move |c| {
        let total: i64 = c.query_row(&format!("SELECT count(*) FROM messages m WHERE {w2}"), params_from_iter(a2.iter()), |r| r.get(0))?;
        let rows: Vec<Row> = c.prepare(&format!("SELECT {ROW_COLS} FROM messages m WHERE {w2} ORDER BY {order} LIMIT {per} OFFSET {}", (page_n - 1) * per))?
            .query_map(params_from_iter(a2.iter()), |r| row(r, false))?.collect::<rusqlite::Result<_>>()?;
        Ok((rows, total))
    })
    .await?;
    let body = html! {
        div.wide {
            div.kicker { (kicker) }
            h1 { (title) }
            (intro)
            p.small { (fmt_n(total)) " " (t("messages", "pesan")) }
            (list(&rows, None, u))
            (pager(&base, page_n, total, per))
        }
    };
    Ok(page(&title, Some(u), "read", body))
}

pub async fn day(State(st): State<AppState>, Reader(u): Reader, Path(date): Path<String>, Query(p): Query<PageQ>) -> Result<Markup, AppError> {
    if date.len() != 10 { return Err(AppError("bad date".into())); }
    let d = date.clone();
    let (prev, next) = blocking(&st, move |c| {
        let prev: Option<String> = c.query_row("SELECT max(date) FROM messages WHERE date < ?1", [&d], |r| r.get(0))?;
        let next: Option<String> = c.query_row("SELECT min(date) FROM messages WHERE date > ?1", [&d], |r| r.get(0))?;
        Ok((prev, next))
    }).await?;
    let intro = html! { div.pager {
        @if let Some(p) = &prev { a href=(format!("/read/day/{p}")) { "← " (nice_date(p)) } }
        a href=(format!("/read/month/{}", &date[..7])) { (t("Whole month", "Sebulan penuh")) }
        @if let Some(n) = &next { a href=(format!("/read/day/{n}")) { (nice_date(n)) " →" } }
    } };
    listing(&st, &u, nice_date(&date), html! { (kicker_home()) " / " (t("day", "hari")) }, intro, "m.date = ? AND m.collection != 'iri-l'".into(), vec![date.clone().into()], "m.id", p.page.unwrap_or(1).max(1), format!("/read/day/{date}")).await
}

pub async fn month(State(st): State<AppState>, Reader(u): Reader, Path(ym): Path<String>) -> Result<Markup, AppError> {
    if ym.len() != 7 { return Err(AppError("bad month".into())); }
    let y2 = ym.clone();
    let days: Vec<(String, i64)> = blocking(&st, move |c| {
        c.prepare("SELECT date, count(*) FROM messages WHERE ym = ?1 AND counted = 1 AND date IS NOT NULL GROUP BY date ORDER BY date")?.query_map([y2], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
    }).await?;
    let max = days.iter().map(|d| d.1).max().unwrap_or(1) as f64;
    let y: i32 = ym[..4].parse().unwrap_or(1990);
    let mo: u32 = ym[5..7].parse().unwrap_or(1);
    let first = chrono::NaiveDate::from_ymd_opt(y, mo, 1).unwrap();
    use chrono::Datelike;
    let offset = first.weekday().num_days_from_monday() as usize;
    let ndays = (if mo == 12 { chrono::NaiveDate::from_ymd_opt(y + 1, 1, 1) } else { chrono::NaiveDate::from_ymd_opt(y, mo + 1, 1) }).unwrap().signed_duration_since(first).num_days() as u32;
    let prev_m = if mo == 1 { format!("{}-12", y - 1) } else { format!("{y}-{:02}", mo - 1) };
    let next_m = if mo == 12 { format!("{}-01", y + 1) } else { format!("{y}-{:02}", mo + 1) };
    let body = html! {
        div.wide {
            div.kicker { (kicker_home()) " / " (t("month", "bulan")) }
            h1 { (nice_date(&ym)) }
            div.pager { a href=(format!("/read/month/{prev_m}")) { "← " (charts::nice_ym(&prev_m)) } a href=(format!("/read/month/{next_m}")) { (charts::nice_ym(&next_m)) " →" } }
            p.small { (fmt_n(days.iter().map(|d| d.1).sum())) " " (t("messages (duplicate copies not counted). Pick a day.", "pesan (salinan ganda tidak dihitung). Pilih satu hari.")) }
            div style="display:grid;grid-template-columns:repeat(7,1fr);gap:6px;max-width:640px;font:13px var(--sans)" {
                @for d in i18n::weekdays() { div.small style="text-align:center" { (d) } }
                @for _ in 0..offset { div {} }
                @for dd in 1..=ndays {
                    @let k = format!("{ym}-{dd:02}");
                    @match days.iter().find(|x| x.0 == k) {
                        Some((_, n)) => {
                            @let a = ((*n as f64 / max).sqrt() * 90.0).round();
                            a href=(format!("/read/day/{k}")) style=(format!("display:block;padding:10px 6px;border-radius:6px;text-decoration:none;background:color-mix(in srgb, var(--accent) {a}%, var(--chip));color:{}", if a > 55.0 { "var(--paper)" } else { "var(--ink)" })) {
                                b { (dd) } br; span style="font-size:11px" { (n) " " (t("msgs", "psn")) }
                            }
                        }
                        None => div style="padding:10px 6px;border-radius:6px;background:var(--chip);opacity:.4" { (dd) br; span style="font-size:11px" { (t("none captured", "tak ada")) } }
                    }
                }
            }
        }
    };
    Ok(page(&nice_date(&ym), Some(&u), "read", body))
}

pub async fn thread(State(st): State<AppState>, Reader(u): Reader, Path(id): Path<String>, Query(p): Query<PageQ>) -> Result<Markup, AppError> {
    let intro = html! { p.small { (t("Messages with the same subject, a reply among them, no more than 45 days apart. The thread is named after its first message, ", "Pesan-pesan dengan subjek yang sama, salah satunya balasan, berjarak tidak lebih dari 45 hari. Utas ini dinamai menurut pesan pertamanya, ")) a href=(format!("/read/m/{id}")) { (id) } "." } };
    listing(&st, &u, t("Thread", "Utas").into(), html! { (kicker_home()) " / " (t("thread", "utas")) }, intro, "m.thread_id = ?".into(), vec![id.clone().into()], "m.date IS NULL, m.date, m.id", p.page.unwrap_or(1).max(1), format!("/read/thread/{}", urlencoding::encode(&id))).await
}

/// The messages the database's folder listings name but no capture holds.
pub async fn lost(State(st): State<AppState>, Reader(u): Reader) -> Result<Markup, AppError> {
    let rows: Vec<(String, Option<String>, i64, String, String, String, String, Option<String>)> = blocking(&st, |c| {
        c.prepare("SELECT ak_id, date, num, size, title, src, ts, maybe FROM lost ORDER BY ak_id")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()
    })
    .await?;
    let cov = st.a("coverage").clone();
    let mut by_year: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for r in &rows {
        *by_year.entry(r.0.get(3..7).unwrap_or("").to_string()).or_default() += 1;
    }
    let days = fmt_n(cov["days"]["listed"].as_i64().unwrap_or(0));
    let named = fmt_n(cov["listed_messages"].as_i64().unwrap_or(0));
    let n = fmt_n(rows.len() as i64);
    let body = html! {
        div.wide {
            div.kicker { (kicker_home()) " / " (t("lost messages", "pesan yang hilang")) }
            h1 { (t("Lost messages", "Pesan yang hilang")) }
            p.lede style="max-width:760px" { (tf(
                || format!("The web servers that hosted the database showed each day's folder as a listing: every file in it, its size and the first 23 characters of its title. The Internet Archive kept those listings for {days} days. They name {named} messages; these {n} are the ones no capture holds. A title ending in \u{201c}>\u{201d} was cut short by the listing."),
                || format!("Server web yang menampung basis data menampilkan folder setiap hari sebagai daftar: setiap berkas di dalamnya, ukurannya dan 23 karakter pertama judulnya. Internet Archive menyimpan daftar itu untuk {days} hari. Daftar itu menyebut {named} pesan; {n} pesan ini tidak terekam sama sekali. Judul yang berakhir dengan \u{201c}>\u{201d} terpotong oleh daftar."))) }
            p.small { @for (y, n) in &by_year { (y) ": " (n) " \u{b7} " } (t("\u{201c}Possible copy\u{201d} is a message in the indopubs.com list archives from the same days whose subject starts with the same words; check it before citing.", "\u{201c}Kemungkinan salinan\u{201d} adalah pesan dalam arsip milis indopubs.com dari hari-hari yang sama yang subjeknya diawali kata-kata yang sama; periksa sebelum mengutip.")) }
            table.t {
                thead { tr { th { "ID" } th { (t("Date", "Tanggal")) } th { (t("Start of the title", "Awal judul")) } th.n { (t("Size", "Ukuran")) } th { (t("Listing", "Daftar")) } th { (t("Possible copy", "Kemungkinan salinan")) } } }
                tbody { @for (ak, date, _num, size, title, src, ts, maybe) in &rows {
                    tr {
                        td style="white-space:nowrap" { (ak) }
                        td style="white-space:nowrap" { (date.as_deref().map(nice_date).unwrap_or_default()) }
                        td { (title) }
                        td.n { (size) }
                        td.small { (crate::listings::source_name(src)) ", " (t("captured", "direkam")) " " (ts.get(..4).unwrap_or("")) }
                        td.small { @if let Some(m) = maybe { @for a in m.split(',') { a href=(format!("/read/m/{a}")) { (a) } " " } } }
                    }
                } }
            }
        }
    };
    Ok(page(t("Lost messages", "Pesan yang hilang"), Some(&u), "read", body))
}

pub async fn topics(State(st): State<AppState>, Reader(u): Reader) -> Result<Markup, AppError> {
    let tp = st.a("topics").clone();
    let lst = tp["topics"].as_array().cloned().unwrap_or_default();
    let (nm, nt) = (fmt_n(tp["n_messages"].as_i64().unwrap_or(0)), fmt_n(tp["n_topics"].as_i64().unwrap_or(0)));
    let body = html! {
        div.wide {
            div.kicker { (kicker_home()) " / " (t("conference topics", "topik konferensi")) }
            h1 { (t("Conference topics", "Topik konferensi")) }
            p.lede style="max-width:760px" { (tf(
                || format!("Until 1999 the database was fed by an IGC conference. A message there either opened a topic or was filed as a reply \u{201c}(r)\u{201d} under one, and a reply carried the topic's name instead of a subject of its own. The first 57 topics were set up on 8 and 9 October 1990 (\u{201c}Law & Liberty\u{201d}, \u{201c}Religion\u{201d}, \u{201c}Area Studies: East Timor\u{201d}); later ones were opened by the first message of a series. {nm} messages sit under {nt} topics."),
                || format!("Hingga 1999 basis data diisi dari sebuah konferensi IGC. Sebuah pesan di sana membuka topik atau diajukan sebagai balasan \u{201c}(r)\u{201d} di bawahnya, dan balasan membawa nama topik itu, bukan subjeknya sendiri. 57 topik pertama dibuat pada 8 dan 9 Oktober 1990 (\u{201c}Law & Liberty\u{201d}, \u{201c}Religion\u{201d}, \u{201c}Area Studies: East Timor\u{201d}); topik selanjutnya dibuka oleh pesan pertama sebuah rangkaian. {nm} pesan berada di bawah {nt} topik."))) }
            table.t {
                thead { tr { th { (t("Topic", "Topik")) } th.n { (t("Messages", "Pesan")) } th { (t("First", "Pertama")) } th { (t("Last", "Terakhir")) } } }
                tbody { @for x in &lst {
                    tr {
                        td { a href=(format!("/read/topic?name={}", urlencoding::encode(x["name"].as_str().unwrap_or("")))) { (x["name"].as_str().unwrap_or("")) } }
                        td.n { (fmt_n(x["n"].as_i64().unwrap_or(0))) }
                        td.small { (x["first"].as_str().unwrap_or("")) } td.small { (x["last"].as_str().unwrap_or("")) }
                    }
                } }
            }
        }
    };
    Ok(page(t("Conference topics", "Topik konferensi"), Some(&u), "read", body))
}

#[derive(Deserialize)]
pub struct TopicQ {
    #[serde(default)]
    pub name: String,
    pub page: Option<i64>,
}

pub async fn topic(State(st): State<AppState>, Reader(u): Reader, Query(q): Query<TopicQ>) -> Result<Markup, AppError> {
    let intro = html! { p.small { (t("Replies filed under this conference topic, oldest first. Their titles come from the first lines of each text; the subject line only names the topic. ", "Balasan yang diajukan di bawah topik konferensi ini, dari yang terlama. Judulnya diambil dari baris pertama setiap teks; baris subjek hanya menyebut topiknya. ")) a href="/read/topics" { (t("All topics", "Semua topik")) } "." } };
    listing(&st, &u, q.name.clone(), html! { (kicker_home()) " / " a href="/read/topics" { (t("topics", "topik")) } }, intro, "m.topic = ? AND m.counted = 1".into(), vec![q.name.clone().into()], "m.date IS NULL, m.date, m.id", q.page.unwrap_or(1).max(1), format!("/read/topic?name={}", urlencoding::encode(&q.name))).await
}

pub async fn source(State(st): State<AppState>, Reader(u): Reader, Path(code): Path<String>, Query(p): Query<PageQ>) -> Result<Markup, AppError> {
    let name = sources::BY_CODE.get(code.as_str()).map(|s| format!("{} ({})", s.name, i18n::category_label(s.category))).unwrap_or_else(|| code.clone());
    listing(&st, &u, name, html! { (kicker_home()) " / " (t("source", "sumber")) " " (code) }, html! { p.small { (t("Messages whose subject, archive link or opening lines name this source. Duplicate copies hidden.", "Pesan yang subjek, tautan arsip atau baris pembukanya menyebut sumber ini. Salinan ganda disembunyikan.")) } },
        "m.source_code = ? AND m.counted = 1".into(), vec![code.clone().into()], "m.date IS NULL, m.date, m.id", p.page.unwrap_or(1).max(1), format!("/read/source/{}", urlencoding::encode(&code))).await
}

#[derive(Deserialize)]
pub struct ThemeQ {
    pub page: Option<i64>,
    #[serde(default)]
    pub all: String,
}

pub async fn theme(State(st): State<AppState>, Reader(u): Reader, Path(slug): Path<String>, Query(p): Query<ThemeQ>) -> Result<Markup, AppError> {
    let th = lexicon::THEMES.iter().find(|x| x.slug == slug);
    let title = th.map(|x| i18n::theme_name(x.slug, x.name)).unwrap_or_else(|| slug.clone());
    let all = !p.all.is_empty();
    let intro = html! { @if let Some(th) = th {
        p { (i18n::theme_desc(th.slug, th.description)) "." }
        @if all {
            p.small { (t("Every message that mentions the theme: one of its terms and two of its words in all, or a term in the title. Many only touch on it. ", "Setiap pesan yang menyebut tema ini: salah satu istilahnya dan dua katanya sekaligus, atau satu istilah di judul. Banyak yang hanya menyinggungnya. ")) a href=(format!("/read/theme/{slug}")) { (t("Show only the messages about it", "Tampilkan hanya pesan yang membahasnya")) } "." }
        } @else {
            p.small { (t("Messages the model judges to be about the theme (precision measured on a sample of fifty; see ", "Pesan yang dinilai model membahas tema ini (presisi diukur pada sampel lima puluh pesan; lihat ")) a href="/analysis/themes" { (t("Themes", "Tema")) } "). " a href=(format!("/read/theme/{slug}?all=1")) { (t("Show every message that mentions it", "Tampilkan setiap pesan yang menyebutnya")) } "." }
        }
        p.small { (t("Terms", "Istilah")) ": " @for w in th.terms { span.chip { (w) } } }
    } };
    let cond = if all { "" } else { " AND mt.about = 1" };
    let base = if all { format!("/read/theme/{slug}?all=1") } else { format!("/read/theme/{slug}") };
    listing(&st, &u, title, html! { (kicker_home()) " / " (t("theme", "tema")) }, intro,
        format!("m.counted = 1 AND m.id IN (SELECT mt.msg_id FROM message_themes mt JOIN themes t ON t.id = mt.theme_id WHERE t.slug = ?{cond})"), vec![slug.clone().into()], "m.date IS NULL, m.date, m.id", p.page.unwrap_or(1).max(1), base).await
}

pub async fn entity(State(st): State<AppState>, Reader(u): Reader, Path(slug): Path<String>, Query(p): Query<PageQ>) -> Result<Markup, AppError> {
    let e = lexicon::ENTITIES.iter().find(|e| e.slug == slug);
    let title = e.map(|e| e.name.to_string()).unwrap_or_else(|| slug.clone());
    let intro = html! { @if let Some(e) = e { p.small { (t("Matched on", "Dicocokkan dengan")) ": " @for a in e.aliases { span.chip { (a) } } " " (t("(whole words, any case). Short names can catch other people of the same name.", "(kata utuh, huruf besar atau kecil). Nama pendek dapat menjaring orang lain dengan nama yang sama.")) } } };
    listing(&st, &u, title, html! { (kicker_home()) " / " (t("name", "nama")) }, intro,
        "m.counted = 1 AND m.id IN (SELECT me.msg_id FROM message_entities me JOIN entities e ON e.id = me.entity_id WHERE e.slug = ?)".into(), vec![slug.clone().into()], "m.date IS NULL, m.date, m.id", p.page.unwrap_or(1).max(1), format!("/read/entity/{slug}")).await
}

pub async fn event(State(st): State<AppState>, Reader(u): Reader, Path(slug): Path<String>, Query(p): Query<PageQ>) -> Result<Response, AppError> {
    let ev = match lexicon::EVENTS.iter().find(|e| e.slug == slug) { Some(e) => e, None => return Ok(StatusCode::NOT_FOUND.into_response()) };
    let intro = html! {
        p { (nice_date(ev.date)) ". " (i18n::event_desc(ev.slug, ev.description)) }
        p.small { (t("Messages from three days before to sixty days after, matching the search ", "Pesan dari tiga hari sebelum hingga enam puluh hari sesudahnya yang cocok dengan pencarian ")) code { (ev.query) } (t(", oldest first. ", ", dari yang terlama. "))
            a href=(format!("/analysis/events#{}", ev.slug)) { (t("How the list reacted", "Bagaimana milis menanggapinya")) } "." }
    };
    Ok(listing(&st, &u, i18n::event_name(ev.slug, ev.name), html! { (kicker_home()) " / " (t("event", "peristiwa")) }, intro,
        "m.counted = 1 AND m.date BETWEEN ? AND ? AND m.id IN (SELECT rowid FROM fts WHERE fts MATCH ?)".into(),
        vec![shift(ev.date, -3).into(), shift(ev.date, 60).into(), ev.query.to_string().into()], "m.date IS NULL, m.date, m.id", p.page.unwrap_or(1).max(1), format!("/read/event/{slug}")).await?.into_response())
}

fn shift(d: &str, days: i64) -> String {
    chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").map(|x| (x + chrono::Duration::days(days)).format("%Y-%m-%d").to_string()).unwrap_or_else(|_| d.to_string())
}

#[derive(Deserialize)]
pub struct CollQ {
    pub page: Option<i64>,
    #[serde(default)]
    pub tier: String,
}

pub async fn collection(State(st): State<AppState>, Reader(u): Reader, Path(slug): Path<String>, Query(p): Query<CollQ>) -> Result<Response, AppError> {
    let focused = p.tier == "3";
    let (title, intro, wh): (&str, &str, &str) = match slug.as_str() {
        "may-1998" if focused => (t("May 1998: the sexual violence (focused)", "Mei 1998: kekerasan seksual (terfokus)"), t("Tier 3: messages from 13 May 1998 on that discuss the sexual violence of the riots in two or more paragraphs, or under a title about it, and mention it less often for other places. A keyword net, not a verdict: read each message.", "Tingkat 3: pesan sejak 13 Mei 1998 yang membahas kekerasan seksual dalam kerusuhan di dua paragraf atau lebih, atau di bawah judul tentangnya, dan lebih jarang menyebutnya untuk tempat lain. Ini jaring kata kunci, bukan putusan: bacalah setiap pesan."), "m.may98_tier = 3 AND m.counted = 1"),
        "may-1998" => (t("May 1998: riots and sexual violence", "Mei 1998: kerusuhan dan kekerasan seksual"), t("Tiers 2 and 3: messages from 13 May 1998 on in which rape or sexual violence appears in the same paragraph as the riots, Chinese Indonesians or the inquiries (TGPF, Tim Relawan, Komnas Perempuan, Ita Martadinata). A keyword net, not a verdict: read each message.", "Tingkat 2 dan 3: pesan sejak 13 Mei 1998 yang menyebut perkosaan atau kekerasan seksual dalam paragraf yang sama dengan kerusuhan, warga Tionghoa atau penyelidikan (TGPF, Tim Relawan, Komnas Perempuan, Ita Martadinata). Ini jaring kata kunci, bukan putusan: bacalah setiap pesan."), "m.may98_tier >= 2 AND m.counted = 1"),
        "sexual-violence" => (t("Sexual violence, all years", "Kekerasan seksual, semua tahun"), t("Every message that mentions rape or sexual violence, including East Timor, Aceh and Papua. A keyword net.", "Setiap pesan yang menyebut perkosaan atau kekerasan seksual, termasuk di Timor Timur, Aceh dan Papua. Jaring kata kunci."), "m.sv_flag = 1 AND m.counted = 1"),
        "iri-l" => ("IRI-L", t("An unrelated list whose archive sat beside apakabar's at indopubs.com (only a few dozen of its messages mention Indonesia). Kept for completeness and left out of every figure.", "Milis lain yang arsipnya berada di samping arsip apakabar di indopubs.com (hanya beberapa puluh pesannya menyebut Indonesia). Disimpan demi kelengkapan dan tidak dihitung dalam angka mana pun."), "m.collection = 'iri-l'"),
        "macdougall" => (t("MacDougall's own notes and summaries", "Catatan dan ringkasan MacDougall sendiri"), t("Messages without an outlet code, posted from the list's own addresses: mostly MacDougall's summaries, notices and early conference postings.", "Pesan tanpa kode sumber yang dikirim dari alamat milis sendiri: kebanyakan ringkasan, pengumuman dan kiriman awal konferensi oleh MacDougall."), "m.source_code IS NULL AND m.msg_type = 'other' AND lower(m.sender) LIKE 'apakabar@%' AND m.counted = 1"),
        "statements" => (t("Statements, petitions and appeals", "Pernyataan, petisi dan seruan"), t("Press releases, statements, urgent actions, petitions and open letters, by subject line.", "Siaran pers, pernyataan, aksi mendesak, petisi dan surat terbuka, menurut baris subjek."), "m.msg_type = 'statement' AND m.counted = 1"),
        "underground" => (t("Underground press", "Pers bawah tanah"), t("SiaR, Xpos, Kabar dari PIJAR and other unlicensed outlets, circulated by e-mail under the New Order.", "SiaR, Xpos, Kabar dari PIJAR dan media tanpa izin lainnya, yang beredar lewat surel di masa Orde Baru."), "m.source_cat = 'alternative' AND m.counted = 1"),
        "literary" => (t("Poems, humour and stories", "Puisi, humor dan cerita"), t("Subjects naming poems, jokes or stories.", "Subjek yang menyebut puisi, lelucon atau cerita."), "m.msg_type = 'literary' AND m.counted = 1"),
        "gerwani" => return Ok(Redirect::to("/read/search?q=gerwani&sort=date").into_response()),
        _ => return Ok(StatusCode::NOT_FOUND.into_response()),
    };
    let review_note = slug == "may-1998" && !u.can_see_sensitive();
    let base = if focused { format!("/read/collection/{slug}?tier=3") } else { format!("/read/collection/{slug}") };
    Ok(listing(&st, &u, title.into(), html! { (kicker_home()) " / " (t("collection", "koleksi")) }, html! {
        p { (intro) }
        @if review_note { p.small { (t("Messages that may name survivors are shown only to approved researchers until an editor has reviewed them, so this list is shorter for readers.", "Pesan yang mungkin menyebut nama penyintas hanya ditampilkan bagi peneliti yang disetujui sampai editor meninjaunya, sehingga daftar ini lebih pendek bagi pembaca.")) } }
    }, wh.into(), vec![], "m.date IS NULL, m.date, m.id", p.page.unwrap_or(1).max(1), base).await?.into_response())
}

#[cfg(test)]
mod tests {
    use super::{fts_query, mask_emails};
    #[test]
    fn queries() {
        assert_eq!(fts_query("perkosaan").unwrap(), "(\"perkosaan\"* OR stems:\"perkosa\")");
        assert_eq!(fts_query("perang").unwrap(), "\"perang\"*");
        assert_eq!(fts_query("\"tim relawan\" -pki").unwrap(), "\"tim relawan\" NOT \"pki\"*");
        assert_eq!(fts_query("timor OR timtim").unwrap(), "(\"timor\"* OR \"timtim\"*)");
        assert!(fts_query("-pki").is_none());
    }
    #[test]
    fn masking() {
        assert_eq!(mask_emails("From: Budi <budi.s@igc.apc.org>"), "From: Budi <\u{2022}\u{2022}\u{2022}@igc.apc.org>");
        assert_eq!(mask_emails("apakabar@clark.net and indonesia-l@igc.org"), "apakabar@clark.net and indonesia-l@igc.org");
        assert_eq!(mask_emails("no address here"), "no address here");
    }
}
