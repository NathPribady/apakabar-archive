//! Public pages: the analytical essay (homepage), the data pages behind it, the catalogue of
//! titles, the about page, and the login and registration forms. Everything in English and
//! Indonesian; the essay itself is read from content/home.en.md and content/home.id.md, so it
//! can be rewritten without touching the program.

use super::auth::{self, MaybeUser};
use super::charts::{self, MonthBar, Series};
use super::i18n::{self, dec, is_id, pct1, t, tf};
use super::layout::{csrf_field, fmt_n, nice_date, page, pct};
use super::security::{self, ClientIp};
use super::{blocking, site, AppError, AppState};
use crate::{lexicon, sources};
use axum::{
    extract::{Form, Query, State},
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use maud::{html, Markup, PreEscaped};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

fn i(v: &Value) -> i64 {
    v.as_i64().unwrap_or(0)
}
fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}
fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}
fn arr(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

/// Year shares of Indonesian-language messages, keyed by year.
fn share_id(st: &AppState, year: &str) -> f64 {
    arr(&st.a("monthly")["years"]).iter().find(|y| s(&y["year"]) == year).map(|y| f(&y["share_id"])).unwrap_or(0.0)
}

/// Share of a year's messages about a theme (`field` = "share") or mentioning it ("mention_share").
fn theme_year(st: &AppState, slug: &str, year: &str, field: &str) -> f64 {
    arr(&st.a("themes")["themes"])
        .iter()
        .find(|x| s(&x["slug"]) == slug)
        .and_then(|x| arr(&x["yearly"]).into_iter().find(|y| s(&y["year"]) == year).map(|y| f(&y[field])))
        .unwrap_or(0.0)
}

fn theme_share(st: &AppState, slug: &str, year: &str) -> f64 {
    theme_year(st, slug, year, "share")
}

/// A precision as a percentage; one decimal when rounding would hide that it is under 90%.
fn pct_p(p: f64) -> String {
    if p < 0.9 && (p * 100.0).round() >= 90.0 { pct1(p) } else { format!("{}%", dec(p * 100.0, 0)) }
}

/// How many hand-checked messages the about-model was fitted on (from src/model.rs).
const MODEL_PAIRS: &str = "914";

fn key_events() -> Vec<(&'static str, &'static str)> {
    vec![
        ("1991-11-12", t("Santa Cruz massacre", "Pembantaian Santa Cruz")),
        ("1994-06-21", t("Tempo, Editor and DeTIK banned", "Tempo, Editor dan DeTIK dibredel")),
        ("1996-07-27", t("Attack on the PDI headquarters", "Penyerbuan kantor PDI")),
        ("1997-08-14", t("Rupiah floated", "Rupiah diambangkan")),
        ("1998-05-13", t("May riots; Soeharto resigns on 21 May", "Kerusuhan Mei; Soeharto mundur pada 21 Mei")),
        ("1999-08-30", t("East Timor referendum", "Referendum Timor Timur")),
        ("2001-07-23", t("Megawati becomes president", "Megawati menjadi presiden")),
    ]
}

fn monthly_chart(st: &AppState, with_events: bool) -> Markup {
    let months: Vec<MonthBar> = arr(&st.a("monthly")["months"])
        .iter()
        .map(|m| MonthBar { ym: s(&m["ym"]), parts: vec![("--id", i(&m["id"])), ("--en", i(&m["en"])), ("--mixed", i(&m["mixed"])), ("--other", i(&m["other"]))] })
        .collect();
    let thin: Vec<String> = arr(&st.a("monthly")["thin"]).iter().map(s).collect();
    let evs: Vec<(String, String)> = if with_events { key_events().iter().map(|(d, n)| (d[..7].to_string(), n.to_string())).collect() } else { vec![] };
    charts::monthly_stacked(&months, &[("--id", t("Indonesian", "Indonesia")), ("--en", t("English", "Inggris")), ("--mixed", t("Mixed", "Campuran")), ("--other", t("Other", "Lainnya"))], &evs, &thin, None)
}

/// Calendar days and days without a folder over a span of months, from the coverage analysis.
fn folder_days(st: &AppState, from: &str, to: &str) -> (i64, i64) {
    let (mut d, mut nf) = (0i64, 0i64);
    for m in arr(&st.a("coverage")["months"]) {
        let ym = s(&m["ym"]);
        if ym.as_str() >= from && ym.as_str() <= to {
            d += i(&m["days"]);
            nf += i(&m["days_nofolder"]);
        }
    }
    (d, nf)
}

/// Coverage (captured / held) over a span of months.
fn coverage_share(st: &AppState, from: &str, to: &str) -> (i64, i64, f64) {
    let (mut c, mut e) = (0i64, 0i64);
    for m in arr(&st.a("coverage")["months"]) {
        let ym = s(&m["ym"]);
        if ym.as_str() >= from && ym.as_str() <= to {
            c += i(&m["captured"]);
            e += i(&m["estimate"]);
        }
    }
    (c, e, if e > 0 { c as f64 / e as f64 } else { 0.0 })
}

/// Precision of a check set from the editors' review (or the first pass when there is no
/// editor review yet): (share, messages judged, who).
pub fn set_precision(st: &AppState, key: &str) -> Option<(f64, i64, &'static str)> {
    // the latest round that has answers ("may98:2/2" before "may98:2")
    let mut sets: Vec<Value> = arr(&st.checks()["sets"]).into_iter().filter(|x| { let k = s(&x["key"]); k == key || k.split('/').next() == Some(key) }).collect();
    sets.sort_by_key(|x| std::cmp::Reverse(i(&x["round"])));
    sets.iter().find_map(set_result)
}

fn set_result(set: &Value) -> Option<(f64, i64, &'static str)> {
    let res = set["results"].as_object()?.clone();
    let mut best: Option<(f64, i64, &'static str)> = None;
    for (who, r) in res.iter() {
        let (y, n) = (i(&r["yes"]), i(&r["no"]));
        if y + n == 0 { continue; }
        let p = y as f64 / (y + n) as f64;
        if who.starts_with("user:") {
            if best.map(|b| b.2 != "editor" || y + n > b.1).unwrap_or(true) { best = Some((p, y + n, "editor")); }
        } else if best.is_none() {
            best = Some((p, y + n, "first pass"));
        }
    }
    best
}

fn who_label(who: &str) -> &'static str {
    if who == "editor" { t("editors", "editor") } else { t("first pass", "pemeriksaan awal") }
}

fn stat(n: String, label: &str) -> Markup {
    html! { div.stat { b { (n) } span { (label) } } }
}

fn read_cta(user: &Option<auth::User>) -> Markup {
    html! {
        div.note {
            @if user.is_some() { (t("You are signed in. ", "Anda sudah masuk. ")) a href="/read" { (t("Open the reading room", "Buka ruang baca")) } (t(" to read every message cited here.", " untuk membaca setiap pesan yang dikutip di sini.")) }
            @else { span.lock { (t("The full texts are in the reading room. ", "Teks lengkapnya ada di ruang baca. ")) } a href="/register" { (t("Register (free, for research)", "Daftar (gratis, untuk penelitian)")) } (t(" or ", " atau ")) a href="/login" { (t("log in", "masuk")) } (t(" to read every message behind these figures.", " untuk membaca setiap pesan di balik angka-angka ini.")) }
        }
    }
}

/// A message title on a public page: a link for signed-in readers, plain text otherwise, and
/// withheld while the message is restricted to researchers.
fn link_msg(st: &AppState, user: &Option<auth::User>, ak: &str, title: &str) -> Markup {
    let can = user.as_ref().map(|u| u.can_see_sensitive()).unwrap_or(false);
    if st.hidden(ak) && !can {
        return html! { span.withheld { (t("Title withheld until an editor has reviewed the message", "Judul ditahan sampai editor meninjau pesan ini")) } };
    }
    html! { @if user.is_some() { a href=(format!("/read/m/{ak}")) { (title) } } @else { (title) } }
}

fn lag_words(e: &Value) -> String {
    match e["first"]["lag_days"].as_i64() {
        Some(0) => t("the same day", "pada hari yang sama").into(),
        Some(1) => t("the next day", "keesokan harinya").into(),
        Some(n) => tf(|| format!("{n} days later"), || format!("{n} hari kemudian")),
        None => t("in no captured message", "tanpa satu pun pesan yang terekam").into(),
    }
}

// ---------------------------------------------------------------- the essay

const HOME_EN: &str = include_str!("../../content/home.en.md");
const HOME_ID: &str = include_str!("../../content/home.id.md");

/// The essay text of the page's language: content/home.<lang>.md if present, else the copy
/// built into the program.
fn essay_source(st: &AppState) -> String {
    let (file, builtin) = if is_id() { ("home.id.md", HOME_ID) } else { ("home.en.md", HOME_EN) };
    std::fs::read_to_string(st.content.join(file)).unwrap_or_else(|_| builtin.to_string())
}

/// Fill the figures into the essay, render its Markdown, and put the charts in place.
fn render_essay(src: &str, values: &HashMap<&'static str, String>, blocks: &HashMap<&'static str, Markup>) -> (String, String) {
    // comments at the top explain the file to its editor; readers do not see them
    let mut text = src.to_string();
    while let (Some(a), Some(b)) = (text.find("<!--"), text.find("-->")) {
        if b < a {
            break;
        }
        text.replace_range(a..b + 3, "");
    }
    for (k, v) in values {
        text = text.replace(&format!("{{{{{k}}}}}"), v);
    }
    let title = text.lines().find_map(|l| l.strip_prefix("# ")).unwrap_or("Apakabar").trim().to_string();
    let mut html_out = String::new();
    pulldown_cmark::html::push_html(&mut html_out, pulldown_cmark::Parser::new_ext(&text, pulldown_cmark::Options::ENABLE_TABLES));
    for (k, b) in blocks {
        let tag = format!("{{{{{k}}}}}");
        let block = b.clone().into_string();
        html_out = html_out.replace(&format!("<p>{tag}</p>"), &block).replace(&tag, &block);
    }
    (title, html_out)
}

pub async fn home(State(st): State<AppState>, MaybeUser(user): MaybeUser) -> Markup {
    let o = st.a("overview").clone();
    let total = i(&o["messages"]);
    let canonical = i(&o["canonical"]);
    let basis = i(&o["basisdata"]);
    let words = i(&o["words"]);
    let cov = st.a("coverage").clone();
    let est = i(&cov["estimate"]);
    let share_orig = f(&cov["share"]);
    let (_, _, q93) = coverage_share(&st, "1993-02", "1993-07");
    let (d93, f93) = folder_days(&st, "1993-02", "1993-07");
    // the months that kept the least, worst first
    let mut lowest: Vec<(f64, String)> = arr(&cov["months"]).iter().filter(|m| f(&m["share"]) < 0.97 && i(&m["estimate"]) >= 100).map(|m| (f(&m["share"]), s(&m["ym"]))).collect();
    lowest.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let lowest_names: Vec<String> = lowest.iter().take(3).map(|(_, ym)| nice_date(ym)).collect();
    let lowest_share = lowest.first().map(|l| l.0).unwrap_or(1.0);
    let thin: Vec<String> = arr(&st.a("monthly")["thin"]).iter().map(|v| nice_date(&s(v))).collect();
    let mil: Vec<f64> = (1991..=2001).map(|y| theme_year(&st, "military", &y.to_string(), "mention_share")).collect();
    let mil_a: Vec<f64> = (1991..=2001).map(|y| theme_share(&st, "military", &y.to_string())).collect();
    let lo = |v: &[f64]| v.iter().cloned().fold(f64::MAX, f64::min);
    let hi = |v: &[f64]| v.iter().cloned().fold(0.0, f64::max);
    let langs = arr(&o["languages"]);
    let lang_n = |code: &str| langs.iter().find(|l| s(&l[0]) == code).map(|l| i(&l[1])).unwrap_or(0);
    let coded = i(&o["coded"]);

    // censorship series
    let qs: Vec<Value> = arr(&st.a("censorship")["quarters"]).into_iter().filter(|q| s(&q["q"]).as_str() >= "1993-Q1").collect();
    let xl: Vec<String> = qs.iter().map(|q| s(&q["q"])).collect();
    let ratio = |k: &str| -> Vec<Option<f64>> { qs.iter().map(|q| { let n = i(&q["news"]); if n >= 30 { Some(i(&q[k]) as f64 / n as f64) } else { None } }).collect() };
    let banned = ratio("banned_alt");
    let peak_banned = qs.iter().zip(banned.iter()).filter_map(|(q, v)| v.map(|v| (s(&q["q"]), v, i(&q["news"])))).fold(("".to_string(), 0.0, 0), |a, b| if b.1 > a.1 { b } else { a });
    let y1997: Vec<f64> = qs.iter().zip(banned.iter()).filter(|(q, _)| s(&q["q"]).starts_with("1997")).filter_map(|(_, v)| *v).collect();
    let avg1997 = if y1997.is_empty() { 0.0 } else { y1997.iter().sum::<f64>() / y1997.len() as f64 };
    let quarter = |q: &str| -> String {
        let (y, n) = (q.get(..4).unwrap_or(""), q.get(6..7).unwrap_or("1"));
        let en = ["first", "second", "third", "fourth"];
        let id = ["pertama", "kedua", "ketiga", "keempat"];
        let k = n.parse::<usize>().unwrap_or(1).clamp(1, 4) - 1;
        tf(|| format!("the {} quarter of {y}", en[k]), || format!("triwulan {} {y}", id[k]))
    };
    let markers: Vec<(usize, String)> = [("1994-Q2", t("press bans", "pembredelan")), ("1996-Q3", t("27 July", "27 Juli")), ("1998-Q2", t("Soeharto falls", "Soeharto jatuh"))]
        .iter().filter_map(|(q, l)| xl.iter().position(|x| x == q).map(|p| (p, l.to_string()))).collect();

    // themes heat
    let themes = arr(&st.a("themes")["themes"]);
    let years: Vec<String> = (1990..=2002).map(|y| y.to_string()).collect();
    let heat_rows: Vec<(String, Option<String>, Vec<f64>, Vec<String>)> = themes.iter().filter(|x| s(&x["slug"]) != "may-1998").map(|x| {
        let ys = arr(&x["yearly"]);
        let vals: Vec<f64> = years.iter().map(|y| ys.iter().find(|z| s(&z["year"]) == *y).map(|z| f(&z["share"])).unwrap_or(0.0)).collect();
        let tips: Vec<String> = vals.iter().zip(years.iter()).map(|(v, y)| format!("{} {}: {} {}", (v * 100.0).round(), y, pct(*v), t("of messages", "dari pesan"))).collect();
        (i18n::theme_name(&s(&x["slug"]), &s(&x["name"])), Some(format!("/analysis/themes#{}", s(&x["slug"]))), vals, tips)
    }).collect();

    // events (selected)
    let evs = arr(&st.a("events")["events"]);
    let ev = |slug: &str| evs.iter().find(|e| s(&e["slug"]) == slug).cloned().unwrap_or(Value::Null);
    let (sc, jul, mar, refm) = (ev("santa-cruz"), ev("27-july"), ev("marsinah"), ev("referendum"));

    // people
    let ents = arr(&st.a("entities")["entities"]);
    let people: Vec<(String, Option<String>, i64, Option<String>)> = ents.iter().filter(|e| s(&e["kind"]) == "person" && s(&e["slug"]) != "macdougall").take(15)
        .map(|e| (s(&e["name"]), Some(format!("/analysis/people#{}", s(&e["slug"]))), i(&e["n"]), None)).collect();

    let may = st.a("may1998").clone();
    let mm = arr(&may["monthly"]);
    let peak_m = mm.iter().max_by_key(|m| i(&m["t2"]) + i(&m["t3"])).map(|m| (s(&m["ym"]), i(&m["t2"]) + i(&m["t3"]))).unwrap_or_default();
    let l65 = arr(&st.a("left1965")["series"]);
    let gerwani_total: i64 = l65.iter().find(|x| s(&x["term"]) == "gerwani").map(|x| x["yearly"].as_object().map(|m| m.values().map(i).sum()).unwrap_or(0)).unwrap_or(0);

    let coverage_months_sentence = if !thin.is_empty() {
        tf(|| format!("In the months the crawler barely visited ({}) the list looks quieter than it was.", thin.join(", ")), || format!("Pada bulan-bulan yang jarang dikunjungi perayap ({}) milis tampak lebih sepi daripada kenyataannya.", thin.join(", ")))
    } else if !lowest_names.is_empty() {
        tf(|| format!("No month lost much: the worst ({}) keep {} of their messages.", lowest_names.join(", "), pct(lowest_share)), || format!("Tidak ada bulan yang kehilangan banyak: yang terburuk ({}) masih menyimpan {} pesannya.", lowest_names.join(", "), pct(lowest_share)))
    } else {
        t("No month lost more than 3% of its messages.", "Tidak ada bulan yang kehilangan lebih dari 3% pesannya.").to_string()
    };
    let marsinah_survival = if q93 >= 0.9995 { t("all of them survive", "semuanya masih ada").to_string() } else { tf(|| format!("{} of them survive", pct(q93)), || format!("{} di antaranya masih ada", pct(q93))) };

    let mut v: HashMap<&'static str, String> = HashMap::new();
    v.insert("total", fmt_n(total));
    v.insert("basis", fmt_n(basis));
    v.insert("est", fmt_n(est));
    v.insert("coverage_share", pct1(share_orig));
    v.insert("lost", fmt_n(i(&cov["lost"])));
    v.insert("days_listed", fmt_n(i(&cov["days"]["listed"])));
    v.insert("days_nofolder", fmt_n(i(&cov["days"]["nofolder"])));
    v.insert("nineties_captured", fmt_n(i(&cov["nineties"]["captured"])));
    v.insert("nineties_estimate", fmt_n(i(&cov["nineties"]["estimate"])));
    v.insert("list_messages", fmt_n(i(&o["list"])));
    v.insert("duplicates", fmt_n(i(&o["duplicates"])));
    v.insert("canonical", fmt_n(canonical));
    v.insert("share_coded", pct(coded as f64 / canonical.max(1) as f64));
    v.insert("coverage_months_sentence", coverage_months_sentence);
    v.insert("english_1991", pct(1.0 - share_id(&st, "1991")));
    v.insert("indonesian_1994", pct(share_id(&st, "1994")));
    v.insert("indonesian_1995", pct(share_id(&st, "1995")));
    v.insert("indonesian_1996", pct(share_id(&st, "1996")));
    v.insert("underground_1997_avg", pct(avg1997));
    v.insert("underground_peak", pct(peak_banned.1));
    v.insert("underground_peak_quarter", quarter(&peak_banned.0));
    v.insert("underground_peak_news", fmt_n(peak_banned.2));
    v.insert("model_pairs", MODEL_PAIRS.to_string());
    for (key, slug, year) in [("et_1992", "east-timor", "1992"), ("et_1999", "east-timor", "1999"), ("economy_1996", "economy", "1996"), ("economy_1998", "economy", "1998"), ("corruption_1995", "corruption", "1995"), ("corruption_1998", "corruption", "1998"), ("communal_1998", "communal-conflict", "1998"), ("communal_1999", "communal-conflict", "1999"), ("aceh_1998", "aceh", "1998"), ("aceh_1999", "aceh", "1999")] {
        v.insert(key, pct(theme_share(&st, slug, year)));
    }
    v.insert("military_mention_lo", pct(lo(&mil)));
    v.insert("military_mention_hi", pct(hi(&mil)));
    v.insert("military_about_lo", pct(lo(&mil_a)));
    v.insert("military_about_hi", pct(hi(&mil_a)));
    v.insert("events_n", lexicon::EVENTS.len().to_string());
    v.insert("lag_santa_cruz", lag_words(&sc));
    v.insert("lag_27_july", lag_words(&jul));
    v.insert("after30_27_july", fmt_n(i(&jul["after30"])));
    v.insert("after30_referendum", fmt_n(i(&refm["after30"])));
    v.insert("lag_marsinah", lag_words(&mar));
    v.insert("days_1993_with_messages", fmt_n(d93 - f93));
    v.insert("days_1993", fmt_n(d93));
    v.insert("marsinah_survival", marsinah_survival);
    v.insert("may_paragraph", fmt_n(i(&may["tiers"]["paragraph"])));
    v.insert("may_focused", fmt_n(i(&may["tiers"]["focused"])));
    v.insert("may_busiest_month", nice_date(&peak_m.0));
    v.insert("may_busiest_n", fmt_n(peak_m.1));
    v.insert("gerwani_total", fmt_n(gerwani_total));

    let thin_note = if arr(&st.a("monthly")["thin"]).is_empty() { "" } else { t(" Grey ticks under the axis flag thin captures.", " Tanda abu-abu di bawah sumbu menandai rekaman yang tipis.") };
    let mut b: HashMap<&'static str, Markup> = HashMap::new();
    b.insert("stats", html! {
        div.stats {
            (stat(fmt_n(basis), t("messages recovered from the apakabar database", "pesan dipulihkan dari basis data apakabar")))
            (stat(pct1(share_orig), &tf(|| format!("of the {} messages the database held, by its own folder listings", fmt_n(est)), || format!("dari {} pesan yang dimuat basis data, menurut daftar foldernya sendiri", fmt_n(est)))))
            (stat(tf(|| format!("{} million", dec(words as f64 / 1e6, 0)), || format!("{} juta", dec(words as f64 / 1e6, 0))), t("words, after removing duplicate copies", "kata, setelah salinan ganda disisihkan")))
            (stat(pct(lang_n("id") as f64 / canonical.max(1) as f64), t("written in Indonesian", "berbahasa Indonesia")))
            (stat(pct(coded as f64 / canonical.max(1) as f64), t("name the outlet they came from", "menyebut media asalnya")))
        }
    });
    b.insert("figure:monthly", html! {
        div.figure {
            h4 { (t("Recovered messages per month, by language", "Pesan yang dipulihkan per bulan, menurut bahasa")) }
            p.sub { (t("Numbered markers: 1 Santa Cruz massacre · 2 press bans · 3 attack on the PDI · 4 rupiah floated · 5 May 1998 · 6 East Timor referendum · 7 Megawati president.", "Penanda bernomor: 1 Pembantaian Santa Cruz · 2 pembredelan pers · 3 penyerbuan kantor PDI · 4 rupiah diambangkan · 5 Mei 1998 · 6 referendum Timor Timur · 7 Megawati menjadi presiden.")) (thin_note) }
            (monthly_chart(&st, true))
            div.src { (tf(|| format!("Source: Apakabar Archive, {} distinct messages.", fmt_n(canonical)), || format!("Sumber: Arsip Apakabar, {} pesan berbeda.", fmt_n(canonical)))) }
        }
    });
    b.insert("figure:underground", html! {
        div.figure {
            h4 { (t("Underground and banned-press share of reposted news", "Bagian media bawah tanah dan pers terlarang dalam berita yang dimuat ulang")) }
            p.sub { (t("SiaR, Xpos, PIJAR, PRD and AJI bulletins, with Tempo Interaktif and D&R from the 1994 bans to 1998. Dashed: foreign press and wires. Share of news with a known outlet, by quarter (quarters under 30 messages left out).", "Buletin SiaR, Xpos, PIJAR, PRD dan AJI, serta Tempo Interaktif dan D&R sejak pembredelan 1994 sampai 1998. Garis putus-putus: pers asing dan kantor berita. Bagian dari berita yang media asalnya diketahui, per triwulan (triwulan dengan kurang dari 30 pesan tidak ditampilkan).")) }
            (charts::lines(&xl, &[
                Series { name: t("Underground", "Bawah tanah").into(), color: "--accent", values: banned.clone(), dashed: false },
                Series { name: t("Foreign and wires", "Asing dan kantor berita").into(), color: "--en", values: ratio("foreign"), dashed: true },
            ], true, &markers, 4, 300.0))
            div.src { (t("More under ", "Selengkapnya di ")) a href="/analysis/sources" { (t("Sources", "Sumber")) } "." }
        }
    });
    b.insert("figure:themes", html! {
        div.figure {
            h4 { (t("Share of each year's messages about each theme", "Bagian pesan setiap tahun yang membahas setiap tema")) }
            p.sub { (t("Shading is relative to each theme's peak year: read along a row, not between rows. Terms under ", "Gradasi warna relatif terhadap tahun puncak setiap tema: baca sepanjang baris, bukan antarbaris. Istilahnya di ")) a href="/analysis/themes" { (t("Themes", "Tema")) } "." }
            (charts::heat_table(&years.iter().map(|y| format!("'{}", &y[2..])).collect::<Vec<_>>(), &heat_rows, true, "--accent"))
        }
    });
    b.insert("figure:people", html! {
        div.figure.inline {
            h4 { (t("People named in the most messages", "Tokoh yang disebut dalam paling banyak pesan")) }
            p.sub { (t("Messages naming each person, 1990–2002 (MacDougall himself left out).", "Jumlah pesan yang menyebut setiap tokoh, 1990–2002 (MacDougall sendiri tidak dihitung).")) }
            (charts::hbars(&people, "--accent2"))
        }
    });
    b.insert("read", read_cta(&user));
    let (title, body_html) = render_essay(&essay_source(&st), &v, &b);
    let body = html! {
        article.essay {
            div.kicker { (t("An analysis of the apakabar lists, 1990–2002", "Analisis milis apakabar, 1990–2002")) }
            (PreEscaped(body_html))
        }
    };
    page(&title, user.as_ref(), "home", body)
}

// ---------------------------------------------------------------- data pages

pub async fn themes(State(st): State<AppState>, MaybeUser(user): MaybeUser) -> Markup {
    let themes = arr(&st.a("themes")["themes"]);
    let all = arr(&st.a("monthly")["months"]);
    let body = html! {
        div.wide {
            div.kicker { (t("Analysis", "Analisis")) }
            h1 { (t("Themes", "Tema")) }
            p.lede style="max-width:720px" { (t("Nineteen themes, each a transparent list of terms. A message mentions a theme when it uses at least one of the theme's terms and two of its words in all, or one term in its title. Common words (in grey) count only beside a term; set phrases (struck through) contain a theme word but mean something else and are ignored.", "Sembilan belas tema, masing-masing berupa daftar istilah yang terbuka. Sebuah pesan menyebut sebuah tema bila memakai paling sedikit satu istilahnya dan dua katanya sekaligus, atau satu istilah di judulnya. Kata umum (abu-abu) hanya dihitung bila berdampingan dengan sebuah istilah; frasa baku (dicoret) memuat kata tema tetapi bermakna lain dan diabaikan.")) }
            p style="max-width:720px" { (tf(|| format!("Many messages only touch on a theme: a digest with one item on Aceh, a speech that names the economy in passing. So a model reads how the theme's words fall in each message (how many, how early, how concentrated, in how many paragraphs) and judges whether the message is about the theme. It is a logistic regression fitted on {MODEL_PAIRS} messages read by hand, and each theme's threshold was set so that nine in ten of the messages above it are about the theme. The lines show the share of each month's messages that are about the theme, from October 1990 to February 2002."), || format!("Banyak pesan hanya menyinggung sebuah tema: rangkuman dengan satu butir tentang Aceh, pidato yang menyebut ekonomi sambil lalu. Karena itu sebuah model membaca bagaimana kata-kata tema tersebar dalam setiap pesan (berapa banyak, seberapa awal, seberapa padat, di berapa paragraf) dan menilai apakah pesan itu membahas tema tersebut. Model ini regresi logistik yang dilatih pada {MODEL_PAIRS} pesan yang dibaca secara manual, dan ambang setiap tema ditetapkan agar sembilan dari sepuluh pesan di atasnya memang membahas tema itu. Garis menunjukkan bagian pesan setiap bulan yang membahas tema itu, dari Oktober 1990 sampai Februari 2002."))) }
            p.small style="max-width:720px" { (t("Precision is measured on fifty messages drawn at random from those the model places in each theme. Because each theme has its own threshold, compare a theme across time rather than the sizes of different themes.", "Presisi diukur pada lima puluh pesan yang diambil acak dari pesan yang ditempatkan model dalam setiap tema. Karena setiap tema punya ambangnya sendiri, bandingkan sebuah tema dari waktu ke waktu, bukan ukuran antartema.")) }
            @for x in &themes {
                @let slug = s(&x["slug"]);
                @let monthly = arr(&x["monthly"]);
                @let vals: Vec<f64> = all.iter().map(|m| { let k = s(&m["ym"]); monthly.iter().find(|z| s(&z["ym"]) == k).map(|z| f(&z["share"])).unwrap_or(0.0) }).collect();
                div.card id=(slug) style="margin:16px 0" {
                    div style="display:flex;justify-content:space-between;gap:16px;flex-wrap:wrap" {
                        div style="max-width:560px" {
                            h3 style="margin:0" { (i18n::theme_name(&slug, &s(&x["name"]))) }
                            p.small style="margin:4px 0" { (i18n::theme_desc(&slug, &s(&x["description"]))) ". "
                                (tf(|| format!("{} messages about it, {} that mention it; busiest month {} ({}).", fmt_n(i(&x["total"])), fmt_n(i(&x["mentions"])), charts::nice_ym(&s(&x["peak"]["ym"])), i(&x["peak"]["n"])), || format!("{} pesan membahasnya, {} menyebutnya; bulan tersibuk {} ({}).", fmt_n(i(&x["total"])), fmt_n(i(&x["mentions"])), charts::nice_ym(&s(&x["peak"]["ym"])), i(&x["peak"]["n"]))))
                                @if let Some((p, n, who)) = set_precision(&st, &format!("about:{slug}")) { " " (t("Precision", "Presisi")) " " b { (pct_p(p)) } " (" (n) " " (t("messages checked", "pesan diperiksa")) ", " (who_label(who)) ")." } }
                            div { @for w in arr(&x["terms"]) { span.chip { (s(&w)) } } @for w in arr(&x["weak"]) { span.chip.weak { (s(&w)) } } @for w in arr(&x["stops"]) { span.chip.stop { (s(&w)) } } }
                        }
                        div { (charts::sparkline(&vals, "--accent")) div.small style="display:flex;justify-content:space-between;width:220px" { span { "1990" } span { "2002" } } }
                    }
                    details { summary { (t("Messages most clearly about the theme", "Pesan yang paling jelas membahas tema ini")) }
                        ul { @for e in arr(&x["examples"]) { li { (link_msg(&st, &user, &s(&e["ak_id"]), &s(&e["title"]))) " " span.small { (s(&e["date"])) " " (s(&e["source"])) } } } }
                        a href=(format!("/read/theme/{slug}")) { (t("All messages about this theme", "Semua pesan yang membahas tema ini")) } " · " a href=(format!("/read/theme/{slug}?all=1")) { (t("every message that mentions it", "setiap pesan yang menyebutnya")) } @if user.is_none() { " " (t("(log in)", "(masuk)")) }
                    }
                }
            }
        }
    };
    page(t("Themes", "Tema"), user.as_ref(), "themes", body)
}

pub async fn sources(State(st): State<AppState>, MaybeUser(user): MaybeUser) -> Markup {
    let src = st.a("sources").clone();
    let cats: Vec<(&str, &str, &str)> = vec![
        ("id_daily", t("Indonesian dailies", "Harian Indonesia"), "--id"),
        ("id_magazine", t("Indonesian magazines", "Majalah Indonesia"), "--mixed"),
        ("id_online", t("Indonesian online", "Media daring Indonesia"), "--teal"),
        ("id_wire", "Antara", "--ink3"),
        ("alternative", t("Underground", "Bawah tanah"), "--ink"),
        ("wire", t("Wire services", "Kantor berita"), "--accent2"),
        ("foreign", t("Foreign press", "Pers asing"), "--en"),
        ("ngo", t("NGOs", "LSM"), "--plum"),
        ("list_rubric", t("Apakabar rubrics", "Rubrik apakabar"), "--rule"),
        ("contributor", t("Contributors", "Kontributor"), "--ink2"),
    ];
    let years: Vec<(String, Vec<(String, i64)>)> = arr(&src["by_year_category"]).iter().map(|y| {
        let m = y["cats"].as_object().cloned().unwrap_or_default();
        (y["year"].to_string(), m.iter().filter(|(k, _)| k.as_str() != "uncoded" && k.as_str() != "other_code").map(|(k, v)| (k.clone(), i(v))).collect())
    }).collect();
    let top: Vec<(String, Option<String>, i64, Option<String>)> = arr(&src["top"]).iter().take(40).map(|x| {
        let code = s(&x["code"]);
        (format!("{} ({})", x["name"].as_str().unwrap_or(&code), code), Some(format!("/read/source/{}", urlencoding::encode(&code))), i(&x["n"]), Some(format!("{}–{}", i(&x["first"]), i(&x["last"]))))
    }).collect();
    let cens = st.a("censorship").clone();
    let by_code = arr(&cens["by_code_year"]);
    let codes: Vec<String> = arr(&cens["banned_alt_codes"]).iter().map(s).collect();
    let yrs: Vec<String> = (1993..=2002).map(|y| y.to_string()).collect();
    let body = html! {
        div.wide {
            div.kicker { (t("Analysis", "Analisis")) }
            h1 { (t("Sources", "Sumber")) }
            p.lede style="max-width:720px" { (t("Where apakabar's texts came from. MacDougall marked most reposts with an outlet code (KMP for ", "Dari mana teks-teks apakabar berasal. MacDougall menandai sebagian besar kiriman ulang dengan kode media (KMP untuk ")) em { "Kompas" } (t(", JKTP for ", ", JKTP untuk ")) em { "The Jakarta Post" } (t("); where the code is missing, the archive link or the opening lines often name the outlet.", "); bila kodenya tidak ada, tautan arsip atau baris pembuka sering menyebut medianya.")) }
            div.figure.inline {
                h4 { (t("Outlets by kind, share of identified messages per year", "Media menurut jenis, bagian dari pesan yang dikenali per tahun")) }
                p.sub { (t("Messages without an identifiable outlet are left out; before 1995 these are the majority (MacDougall's own summaries).", "Pesan yang media asalnya tak dapat dikenali tidak dihitung; sebelum 1995 pesan seperti ini mayoritas (ringkasan MacDougall sendiri).")) }
                (charts::stacked100(&years, &cats))
            }
            div.grid2 {
                div { h3 { (t("Most-cited outlets", "Media yang paling sering dikutip")) } (charts::hbars(&top, "--accent2")) }
                div {
                    h3 { (t("Banned, successor and underground outlets by year", "Media yang dibredel, penerusnya dan media bawah tanah per tahun")) }
                    table.t {
                        thead { tr { th { (t("Code", "Kode")) } @for y in &yrs { th.n { "'" (&y[2..]) } } } }
                        tbody { @for c in &codes {
                            @let row: Vec<i64> = yrs.iter().map(|y| by_code.iter().find(|b| s(&b["code"]) == *c && b["year"].to_string() == *y).map(|b| i(&b["n"])).unwrap_or(0)).collect();
                            @if row.iter().sum::<i64>() > 0 {
                                tr { td { a href=(format!("/read/source/{}", urlencoding::encode(c))) { (c) } br; span.small { (sources::BY_CODE.get(c.as_str()).map(|x| x.name).unwrap_or("")) } } @for n in &row { td.n { @if *n > 0 { (fmt_n(*n)) } } } }
                            }
                        } }
                    }
                }
            }
            (read_cta(&user))
        }
    };
    page(t("Sources", "Sumber"), user.as_ref(), "sources", body)
}

pub async fn events(State(st): State<AppState>, MaybeUser(user): MaybeUser) -> Markup {
    let evs = arr(&st.a("events")["events"]);
    let body = html! {
        div.essay {
            div.kicker { (t("Analysis", "Analisis")) }
            h1 { (t("Events and attention", "Peristiwa dan perhatian")) }
            p.lede { (t("For each event: messages per day from thirty days before to ninety days after (grey before, red after), the first captured message, and the volume in the following thirty days compared with the month before.", "Untuk setiap peristiwa: pesan per hari dari tiga puluh hari sebelum hingga sembilan puluh hari sesudahnya (abu-abu sebelum, merah sesudah), pesan pertama yang terekam, dan jumlah pesan dalam tiga puluh hari berikutnya dibandingkan dengan sebulan sebelumnya.")) }
            p.small { (t("Matching uses a search query per event, shown under each chart; broad queries (elections, the riots) also catch unrelated messages, so their baseline is high. \"Capture\" gives all messages captured in the thirty days after the event, to tell a quiet list from a missing capture.", "Pencocokan memakai satu pencarian per peristiwa, yang tertera di bawah setiap grafik; pencarian yang luas (pemilu, kerusuhan) juga menjaring pesan yang tak berkaitan, sehingga garis dasarnya tinggi. \"Rekaman\" menyebut semua pesan yang terekam dalam tiga puluh hari sesudah peristiwa, untuk membedakan milis yang sepi dari rekaman yang hilang.")) }
            div.timeline {
                @for e in &evs {
                    @let slug = s(&e["slug"]);
                    @let daily: Vec<(i64, i64)> = arr(&e["daily"]).iter().map(|d| (i(&d[0]), i(&d[1]))).collect();
                    div.ev id=(slug) {
                        div.d { (nice_date(&s(&e["date"]))) }
                        h3 style="margin:2px 0 4px" { (i18n::event_name(&slug, &s(&e["name"]))) }
                        p.small style="margin:0 0 6px" { (i18n::event_desc(&slug, &s(&e["description"]))) }
                        div style="max-width:420px" { (charts::event_bars(&daily, -30, 90)) }
                        p.small {
                            (t("First message", "Pesan pertama")) ": " @match e["first"].as_object() {
                                Some(fm) => { b { (lag_words(e)) } " — " (link_msg(&st, &user, fm.get("ak_id").and_then(|v| v.as_str()).unwrap_or(""), fm.get("title").and_then(|v| v.as_str()).unwrap_or(""))) }
                                None => (t("none captured", "tidak ada yang terekam")),
                            }
                            br;
                            (fmt_n(i(&e["after30"]))) " " (t("matching messages in 30 days · capture:", "pesan yang cocok dalam 30 hari · rekaman:")) " " (fmt_n(i(&e["all_after30"]))) " " (t("messages of all kinds", "pesan dari segala jenis"))
                            @if f(&e["baseline_per_day"]) > 0.0 { " · " (dec(f(&e["lift"]), 1)) (t("× the earlier daily rate", "× laju harian sebelumnya")) }
                            br; (t("Query", "Pencarian")) ": " code { (s(&e["query"])) }
                            @if user.is_some() { " · " a href=(format!("/read/event/{slug}")) { (t("read the messages", "baca pesan-pesannya")) } }
                        }
                    }
                }
            }
            (read_cta(&user))
        }
    };
    page(t("Events", "Peristiwa"), user.as_ref(), "events", body)
}

pub async fn people(State(st): State<AppState>, MaybeUser(user): MaybeUser) -> Markup {
    let ents = arr(&st.a("entities")["entities"]);
    let bars = |kind: &str, n: usize| -> Vec<(String, Option<String>, i64, Option<String>)> {
        ents.iter().filter(|e| s(&e["kind"]) == kind).take(n).map(|e| (s(&e["name"]), Some(format!("/read/entity/{}", s(&e["slug"]))), i(&e["n"]), None)).collect()
    };
    let years: Vec<String> = (1990..=2002).map(|y| y.to_string()).collect();
    let rows: Vec<(String, Option<String>, Vec<f64>, Vec<String>)> = ents.iter().filter(|e| s(&e["kind"]) == "person").take(25).map(|e| {
        let y = e["yearly"].as_object().cloned().unwrap_or_default();
        let vals: Vec<f64> = years.iter().map(|k| y.get(k).map(i).unwrap_or(0) as f64).collect();
        let tips: Vec<String> = vals.iter().zip(years.iter()).map(|(v, k)| format!("{} {} {k}", *v as i64, t("in", "pada"))).collect();
        (s(&e["name"]), Some(format!("/read/entity/{}", s(&e["slug"]))), vals, tips)
    }).collect();
    let co = st.a("entities")["cooccurrence"].clone();
    let n_names = lexicon::ENTITIES.len();
    let body = html! {
        div.wide {
            div.kicker { (t("Analysis", "Analisis")) }
            h1 { (t("People, organisations and places", "Tokoh, organisasi dan tempat")) }
            p.lede style="max-width:720px" { (tf(|| format!("Counts of distinct messages naming each person, organisation or place, from a list of {n_names} names and spellings. Short names can catch namesakes; see each list of spellings in the reading room."), || format!("Jumlah pesan berbeda yang menyebut setiap tokoh, organisasi atau tempat, dari daftar {n_names} nama dan ejaannya. Nama pendek dapat menjaring orang lain yang bernama sama; lihat daftar ejaan masing-masing di ruang baca."))) }
            div.figure.inline { h4 { (t("Messages naming each person, by year", "Pesan yang menyebut setiap tokoh, per tahun")) } p.sub { (t("Shading relative to each person's own busiest year; numbers are message counts.", "Gradasi warna relatif terhadap tahun tersibuk setiap tokoh; angkanya adalah jumlah pesan.")) } (charts::heat_table(&years.iter().map(|y| format!("'{}", &y[2..])).collect::<Vec<_>>(), &rows, true, "--accent2")) }
            div.grid3 {
                div { h3 { (t("People", "Tokoh")) } (charts::hbars(&bars("person", 30), "--accent")) }
                div { h3 { (t("Organisations", "Organisasi")) } (charts::hbars(&bars("org", 30), "--accent2")) }
                div { h3 { (t("Places", "Tempat")) } (charts::hbars(&bars("place", 30), "--ink3")) }
            }
            h2 { (t("Named together", "Disebut bersama")) }
            p { (t("Pairs of the 24 most-named people and organisations that appear in the same message most often, in three periods.", "Pasangan dari 24 tokoh dan organisasi yang paling sering disebut, yang paling sering muncul dalam pesan yang sama, dalam tiga periode.")) }
            div.grid3 {
                @for p in ["1990-1997", "1998-1999", "2000-2002"] {
                    div { h3 { (p.replace('-', "–")) }
                        table.t { tbody { @for c in arr(&co[p]).iter().take(15) { tr { td { (s(&c["a"])) " + " (s(&c["b"])) } td.n { (fmt_n(i(&c["n"]))) } } } } }
                    }
                }
            }
            (read_cta(&user))
        }
    };
    page(t("People", "Tokoh"), user.as_ref(), "people", body)
}

pub async fn words(State(st): State<AppState>, MaybeUser(user): MaybeUser) -> Markup {
    let ys = arr(&st.a("distinctive_terms")["years"]);
    let body = html! {
        div.essay {
            div.kicker { (t("Analysis", "Analisis")) }
            h1 { (t("The words of each year", "Kata-kata setiap tahun")) }
            p.lede { (t("Words that are unusually frequent in one year compared with all the others, scored with Dunning's log-likelihood (G²). Each word is counted once per message, so a single long document cannot dominate a year.", "Kata-kata yang luar biasa sering muncul dalam satu tahun dibandingkan tahun-tahun lainnya, dinilai dengan log-likelihood Dunning (G²). Setiap kata dihitung sekali per pesan, sehingga satu dokumen panjang tidak dapat mendominasi sebuah tahun.")) }
            p.small { (t("Stop words, numbers and words seen in fewer than 40 messages are left out. Larger type means a stronger score.", "Kata umum, angka dan kata yang muncul di kurang dari 40 pesan tidak dihitung. Huruf yang lebih besar berarti skor yang lebih kuat.")) }
            div.terms {
                @for y in &ys {
                    @let terms = arr(&y["terms"]);
                    @let max = terms.first().map(|x| f(&x["g2"])).unwrap_or(1.0).max(1.0);
                    b { (y["year"]) }
                    div { @for x in terms.iter().take(24) {
                        span style=(format!("font-size:{:.1}px;margin-right:10px;white-space:nowrap", 12.0 + 9.0 * (f(&x["g2"]) / max).sqrt())) title=(format!("G² {} · {} {}", f(&x["g2"]), i(&x["docs"]), t("messages", "pesan"))) {
                            @if user.is_some() { a href=(format!("/read/search?q={}&from={}-01-01&to={}-12-31", urlencoding::encode(&s(&x["w"])), y["year"], y["year"])) style="text-decoration:none" { (s(&x["w"])) } } @else { (s(&x["w"])) }
                        }
                    } }
                }
            }
            (read_cta(&user))
        }
    };
    page(t("Words", "Kata"), user.as_ref(), "words", body)
}

pub async fn may1998(State(st): State<AppState>, MaybeUser(user): MaybeUser) -> Markup {
    let m = st.a("may1998").clone();
    let tiers = m["tiers"].clone();
    let months: Vec<MonthBar> = arr(&m["monthly"]).iter().map(|x| MonthBar { ym: s(&x["ym"]), parts: vec![("--accent", i(&x["t3"])), ("--accent2", i(&x["t2"])), ("--other", i(&x["t1"]))] }).collect();
    let srcs: Vec<(String, Option<String>, i64, Option<String>)> = arr(&m["sources"]).iter().map(|x| (format!("{} {}", s(&x["code"]), x["name"].as_str().map(|n| format!("({n})")).unwrap_or_default()), None, i(&x["n"]), None)).collect();
    let terms: Vec<(String, Option<String>, i64, Option<String>)> = arr(&m["terms"]).iter().map(|x| (s(&x["term"]), None, i(&x["n"]), None)).collect();
    let (_, _, q98) = coverage_share(&st, "1998-05", "1998-10");
    let prec2 = set_precision(&st, "may98:2");
    let prec3 = set_precision(&st, "may98:3");
    let (t1, t2, t3) = (fmt_n(i(&tiers["mention"])), fmt_n(i(&tiers["paragraph"])), fmt_n(i(&tiers["focused"])));
    let can = user.as_ref().map(|u| u.can_see_sensitive()).unwrap_or(false);
    let body = html! {
        div.essay {
            div.kicker { (t("Case study", "Studi kasus")) }
            h1 { (t("The May 1998 sexual violence on apakabar", "Kekerasan seksual Mei 1998 di apakabar")) }
            p.lede { (t("How the reports of rape during the riots of 13–15 May 1998 reached the list, how they were contested, and how the inquiries were followed.", "Bagaimana laporan perkosaan dalam kerusuhan 13–15 Mei 1998 sampai ke milis, bagaimana laporan itu digugat, dan bagaimana penyelidikannya diikuti.")) }
            p { (PreEscaped(tf(
                || format!("Messages are sorted into three tiers. From 13 May 1998 on, <b>{t1}</b> messages mention rape or sexual violence at all, many of them about East Timor, Aceh or Papua. In <b>{t2}</b> the violence appears in the same paragraph as the riots, Chinese Indonesians or the inquiries (tier 2 and above), and <b>{t3}</b> are mainly about it (tier 3: the violence runs through the text, with twelve or more mentions per thousand words spread over a quarter of its paragraphs, or eight under a title that names it)."),
                || format!("Pesan dibagi menjadi tiga tingkat. Sejak 13 Mei 1998, <b>{t1}</b> pesan menyebut perkosaan atau kekerasan seksual, banyak di antaranya tentang Timor Timur, Aceh atau Papua. Dalam <b>{t2}</b> pesan, kekerasan itu muncul dalam paragraf yang sama dengan kerusuhan, warga Tionghoa atau penyelidikan (tingkat 2 ke atas), dan <b>{t3}</b> pesan terutama membahasnya (tingkat 3: kekerasan itu hadir di sepanjang teks, dengan dua belas sebutan atau lebih per seribu kata yang tersebar di seperempat paragrafnya, atau delapan sebutan di bawah judul yang menyebutnya).")))) }
            p.small { (t("A keyword net: it finds reports, denials, appeals and debate alike, and misses messages that use other words. ", "Sebuah jaring kata kunci: ia menemukan laporan, penyangkalan, seruan dan perdebatan sekaligus, dan melewatkan pesan yang memakai kata lain. "))
                @if let Some((p, n, who)) = prec2 { (tf(|| format!("On a check of {n} messages, {} of tier 2 and above were about the May violence ({}). ", pct_p(p), who_label(who)), || format!("Pada pemeriksaan {n} pesan, {} dari tingkat 2 ke atas memang tentang kekerasan Mei ({}). ", pct_p(p), who_label(who)))) }
                @if let Some((p, n, who)) = prec3 { (tf(|| format!("For tier 3 the share of messages mainly about it was {} ({n} checked, {})", pct_p(p), who_label(who)), || format!("Untuk tingkat 3, bagian pesan yang terutama membahasnya adalah {} ({n} diperiksa, {})", pct_p(p), who_label(who)))) @if p < 0.9 { (t(", below the 90% target: treat the focused list as a starting point and count with tiers 2 and 3 together", ", di bawah target 90%: perlakukan daftar terfokus sebagai titik awal dan hitunglah tingkat 2 dan 3 bersama-sama")) } ". " }
                (tf(|| format!("From May to October 1998, {} of the database's messages survive.", pct(q98)), || format!("Dari Mei sampai Oktober 1998, {} pesan basis data masih ada.", pct(q98)))) }
            @if !can { p.small { (t("Messages that may name survivors or private people are open only to approved researchers, and their titles are withheld below until an editor has reviewed them.", "Pesan yang mungkin menyebut nama penyintas atau orang biasa hanya terbuka bagi peneliti yang disetujui, dan judulnya di bawah ditahan sampai editor meninjaunya.")) } }
            div.figure {
                h4 { (t("Messages per month: focused (dark red), same paragraph (blue), other mentions of sexual violence (grey)", "Pesan per bulan: terfokus (merah tua), paragraf yang sama (biru), sebutan lain tentang kekerasan seksual (abu-abu)")) }
                (charts::monthly_stacked(&months, &[("--accent", t("focused (tier 3)", "terfokus (tingkat 3)")), ("--accent2", t("same paragraph (tier 2)", "paragraf yang sama (tingkat 2)")), ("--other", t("mentions (tier 1)", "sebutan (tingkat 1)"))], &[], &[], None))
            }
            div.grid2 {
                div { h3 { (t("Where the messages came from", "Dari mana pesan-pesan itu berasal")) } p.small { (t("Tiers 2 and 3.", "Tingkat 2 dan 3.")) } (charts::hbars(&srcs, "--accent2")) }
                div { h3 { (t("Words used in them", "Kata-kata yang dipakai")) } p.small { (t("Messages of tiers 2 and 3 containing each word (substring match).", "Pesan tingkat 2 dan 3 yang memuat setiap kata (pencocokan sebagian kata).")) } (charts::hbars(&terms, "--accent")) }
            }
            h2 { (t("The first messages", "Pesan-pesan pertama")) }
            table.t { tbody { @for x in arr(&m["first"]) { tr { td style="white-space:nowrap" { (s(&x["date"])) } td { (link_msg(&st, &user, &s(&x["ak_id"]), &s(&x["title"]))) } td { (s(&x["source"])) } td.small { (t("tier", "tingkat")) " " (i(&x["tier"])) } } } } }
            p { (t("These messages complement the ", "Pesan-pesan ini melengkapi ")) b { (t("May 1998 Sexual Violence Archive", "Arsip Kekerasan Seksual Mei 1998")) } (t(" (Nathanael Pribady), which collects the TGPF report, Tim Relawan documentation, the Indonesian and foreign press, UN records and testimony.", " (Nathanael Pribady), yang menghimpun laporan TGPF, dokumentasi Tim Relawan, pers Indonesia dan asing, catatan PBB serta kesaksian.")) }
            @if user.is_some() { p { a href="/read/collection/may-1998" { (t("Read tiers 2 and 3 →", "Baca tingkat 2 dan 3 →")) } " · " a href="/read/collection/may-1998?tier=3" { (t("the focused messages only →", "pesan terfokus saja →")) } } }
            (read_cta(&user))
        }
    };
    page(t("May 1998", "Mei 1998"), user.as_ref(), "home", body)
}

pub async fn left1965(State(st): State<AppState>, MaybeUser(user): MaybeUser) -> Markup {
    let l = st.a("left1965").clone();
    let years: Vec<String> = (1990..=2002).map(|y| y.to_string()).collect();
    let colors = ["--accent", "--accent2", "--mixed", "--ink2", "--en", "--id", "--ink3", "--other", "--ink"];
    let series: Vec<Series> = arr(&l["series"]).iter().enumerate().filter(|(_, x)| ["gerwani", "pki", "g30s", "pramoedya", "tapol"].contains(&s(&x["term"]).as_str())).map(|(k, x)| {
        let y = x["yearly"].as_object().cloned().unwrap_or_default();
        Series { name: s(&x["label"]), color: colors[k % colors.len()], values: years.iter().map(|yy| Some(y.get(yy).map(i).unwrap_or(0) as f64)).collect(), dashed: k % 2 == 1 }
    }).collect();
    let body = html! {
        div.essay {
            div.kicker { (t("Case study", "Studi kasus")) }
            h1 { (t("Gerwani, the PKI and the memory of 1965", "Gerwani, PKI dan ingatan tentang 1965")) }
            p.lede { (t("How often the list named the women's movement Gerwani, the PKI, the 30 September Movement, Pramoedya and the political prisoners, year by year.", "Seberapa sering milis menyebut gerakan perempuan Gerwani, PKI, Gerakan 30 September, Pramoedya dan para tahanan politik, dari tahun ke tahun.")) }
            div.figure.inline { h4 { (t("Messages naming each term, per year", "Pesan yang menyebut setiap istilah, per tahun")) } (charts::lines(&years, &series, false, &[], 1, 280.0)) div.src { (t("Counts are of distinct messages; ", "Hitungan berupa pesan berbeda; ")) em { "pki" } (t(" also matches inside longer words only when they begin with it.", " juga cocok di dalam kata yang lebih panjang hanya bila kata itu diawali dengannya.")) } }
            h2 { (t("Every message naming Gerwani, in order", "Setiap pesan yang menyebut Gerwani, berurutan")) }
            table.t { tbody { @for x in arr(&l["gerwani_messages"]) { tr { td style="white-space:nowrap" { (s(&x["date"])) } td { (link_msg(&st, &user, &s(&x["ak_id"]), &s(&x["title"]))) } td { (s(&x["source"])) } } } } }
            @if user.is_some() { p { a href="/read/search?q=gerwani&sort=date" { (t("Search all Gerwani messages →", "Cari semua pesan tentang Gerwani →")) } } }
            (read_cta(&user))
        }
    };
    page("1965", user.as_ref(), "home", body)
}

// ---------------------------------------------------------------- catalogue

#[derive(Deserialize, Default)]
pub struct CatQ {
    #[serde(default)]
    ym: String,
    #[serde(default)]
    q: String,
    page: Option<i64>,
}

pub async fn catalogue(State(st): State<AppState>, MaybeUser(user): MaybeUser, Query(q): Query<CatQ>) -> Result<Markup, AppError> {
    let per = 100i64;
    let page_n = q.page.unwrap_or(1).max(1);
    let ym = if q.ym.len() == 7 { q.ym.clone() } else { "1998-05".to_string() };
    let qq = q.q.trim().to_string();
    let ym2 = ym.clone();
    let can = user.as_ref().map(|u| u.can_see_sensitive()).unwrap_or(false);
    // restricted titles are not searchable by those who may not read them
    let vis = if can { "1 = 1" } else { "m.sensitive = 0" };
    let (rows, total) = blocking(&st, move |c| {
        if !qq.is_empty() {
            let fq = format!("title : ({})", qq.split_whitespace().map(|w| format!("\"{}\"*", w.replace('"', ""))).collect::<Vec<_>>().join(" AND "));
            let total: i64 = c.query_row(&format!("SELECT count(*) FROM fts JOIN messages m ON m.id = fts.rowid WHERE fts MATCH ?1 AND m.counted = 1 AND {vis}"), [&fq], |r| r.get(0)).unwrap_or(0);
            let rows: Vec<(String, Option<String>, String, Option<String>, bool)> = c.prepare(&format!("SELECT m.ak_id, m.date, m.title, m.source_code, m.sensitive FROM fts JOIN messages m ON m.id = fts.rowid WHERE fts MATCH ?1 AND m.counted = 1 AND {vis} ORDER BY m.date LIMIT {per} OFFSET {}", (page_n - 1) * per))?
                .query_map([&fq], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, i64>(4)? == 1)))?.collect::<rusqlite::Result<_>>()?;
            Ok((rows, total))
        } else {
            let total: i64 = c.query_row("SELECT count(*) FROM messages WHERE ym = ?1 AND counted = 1", [&ym2], |r| r.get(0))?;
            let rows = c.prepare(&format!("SELECT ak_id, date, title, source_code, sensitive FROM messages WHERE ym = ?1 AND counted = 1 ORDER BY date, id LIMIT {per} OFFSET {}", (page_n - 1) * per))?
                .query_map([&ym2], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, i64>(4)? == 1)))?.collect::<rusqlite::Result<_>>()?;
            Ok((rows, total))
        }
    })
    .await?;
    let pages = (total + per - 1) / per;
    let base = if q.q.trim().is_empty() { format!("/catalogue?ym={ym}") } else { format!("/catalogue?q={}", urlencoding::encode(q.q.trim())) };
    let body = html! {
        div.wide {
            div.kicker { (t("Catalogue", "Katalog")) }
            h1 { (t("Catalogue of titles", "Katalog judul")) }
            p { (t("Titles, dates and outlets of every distinct message, open to all. Where the subject line was cut short or only named a conference topic, the title comes from the first lines of the text. The texts themselves are in the reading room. Titles of messages that may name survivors of sexual violence are withheld until an editor has reviewed them.", "Judul, tanggal dan media asal setiap pesan yang berbeda, terbuka untuk umum. Bila baris subjek terpotong atau hanya menyebut topik konferensi, judulnya diambil dari baris pertama teks. Teksnya sendiri ada di ruang baca. Judul pesan yang mungkin menyebut nama penyintas kekerasan seksual ditahan sampai editor meninjaunya.")) }
            form method="get" action="/catalogue" class="searchbar" style="max-width:720px" {
                input type="search" name="q" value=(q.q) placeholder=(t("Search titles (e.g. Timor, Marsinah, Tempo)", "Cari judul (misalnya Timor, Marsinah, Tempo)"));
                button.btn type="submit" { (t("Search titles", "Cari judul")) }
            }
            div style="margin:16px 0;font:13px var(--sans);line-height:2" {
                @for y in 1990..=2002 {
                    b { (y) } " "
                    @for mo in 1..=12usize { @let k = format!("{y}-{mo:02}"); @if k.as_str() >= "1990-10" && k.as_str() <= "2002-02" { a.chip href=(format!("/catalogue?ym={k}")) style=(if k == ym && q.q.is_empty() { "background:var(--ink);color:var(--paper)" } else { "" }) { (i18n::month_short(mo)) } } }
                    br;
                }
            }
            h2 { @if q.q.trim().is_empty() { (nice_date(&ym)) } @else { (t("Titles matching", "Judul yang cocok dengan")) " “" (q.q.trim()) "”" } " · " (fmt_n(total)) }
            table.t {
                thead { tr { th { (t("Date", "Tanggal")) } th { (t("Title", "Judul")) } th { (t("Source", "Sumber")) } th { "ID" } } }
                tbody { @for (ak, d, title, c, sens) in &rows { tr { td style="white-space:nowrap" { (d.clone().unwrap_or_default()) } td { @if *sens && !can { span.withheld { (t("Title withheld until an editor has reviewed the message", "Judul ditahan sampai editor meninjau pesan ini")) } } @else { (link_msg(&st, &user, ak, title)) } } td { (c.clone().unwrap_or_default()) } td.small { (ak) } } } }
            }
            @if pages > 1 { div.pager { @if page_n > 1 { a href=(format!("{base}&page={}", page_n - 1)) { (t("← Previous", "← Sebelumnya")) } } span.small { (t("Page", "Halaman")) " " (page_n) " " (t("of", "dari")) " " (pages) } @if page_n < pages { a href=(format!("{base}&page={}", page_n + 1)) { (t("Next →", "Berikutnya →")) } } } }
            (read_cta(&user))
        }
    };
    Ok(page(t("Catalogue", "Katalog"), user.as_ref(), "catalogue", body))
}

// ---------------------------------------------------------------- about

pub async fn about(State(st): State<AppState>, MaybeUser(user): MaybeUser) -> Markup {
    let o = st.a("overview").clone();
    let est = fmt_n(i(&st.a("coverage")["estimate"]));
    let contact = st.contact.clone();
    let body = html! {
        article.essay {
            div.kicker { (t("About", "Tentang")) }
            h1 { (t("About the archive", "Tentang arsip ini")) }
            h2 { (t("Apakabar and John A. MacDougall", "Apakabar dan John A. MacDougall")) }
            p { (tf(|| format!("John A. MacDougall ran Indonesia Publications, a small press and news service on Indonesia, from 1984 to 2004. In October 1990 he opened apakabar, a moderated conference on the Institute for Global Communications network, and from it built a family of moderated, bilingual lists that he edited until February 2002. They reached, by his own estimate, some 250,000 readers in 96 countries. His biography speaks of about 175,000 documents across all the lists; the main database, hosted by Hamline University and later by Ohio University Library, held {est} messages by the count of its own folder listings. He died in Maryland in May 2023."),
                    || format!("John A. MacDougall mengelola Indonesia Publications, penerbit kecil dan layanan berita tentang Indonesia, dari 1984 sampai 2004. Pada Oktober 1990 ia membuka apakabar, sebuah konferensi bermoderator di jaringan Institute for Global Communications, dan dari situ membangun sekumpulan milis dwibahasa bermoderator yang ia sunting hingga Februari 2002. Menurut perkiraannya sendiri, milis itu menjangkau sekitar 250.000 pembaca di 96 negara. Biografinya menyebut sekitar 175.000 dokumen di semua milis; basis data utamanya, yang ditampung Hamline University dan kemudian Ohio University Library, memuat {est} pesan menurut hitungan daftar foldernya sendiri. Ia wafat di Maryland pada Mei 2023."))) }
            p { (t("For readers inside Indonesia, apakabar was one of the few places where banned magazines, underground bulletins, NGO statements and foreign reporting could be read side by side, and where Indonesians could argue in public about their own government. The original sites are offline. This archive rebuilds as much of the list as the Internet Archive preserved.", "Bagi pembaca di Indonesia, apakabar adalah salah satu dari sedikit tempat untuk membaca majalah terlarang, buletin bawah tanah, pernyataan LSM dan liputan asing berdampingan, dan tempat orang Indonesia dapat berdebat secara terbuka tentang pemerintahnya sendiri. Situs-situs aslinya sudah tidak aktif. Arsip ini membangun kembali sebanyak mungkin isi milis yang tersimpan di Internet Archive.")) }
            h2 { (t("Terms of use", "Ketentuan penggunaan")) }
            p { (t("Reading-room access is free for research and teaching. Most messages repost articles that remain under copyright, and many name private people. Readers agree to use the texts for research, to quote within fair use, to cite the archive, and not to republish or redistribute full texts. Messages that may name survivors of sexual violence or private people are restricted to approved researchers.", "Akses ke ruang baca gratis untuk penelitian dan pengajaran. Sebagian besar pesan memuat ulang artikel yang masih dilindungi hak cipta, dan banyak yang menyebut nama orang biasa. Pembaca setuju memakai teks untuk penelitian, mengutip dalam batas penggunaan wajar, mencantumkan arsip ini sebagai sumber, dan tidak menerbitkan ulang atau menyebarkan teks lengkap. Pesan yang mungkin menyebut nama penyintas kekerasan seksual atau orang biasa hanya terbuka bagi peneliti yang disetujui.")) }
            h2 id="takedown" { (t("Corrections and takedown", "Koreksi dan penghapusan")) }
            p { (t("Authors, publishers and people named in the messages can ask for a correction, for a message to be restricted, or for its removal. Requests are answered within 30 days.", "Penulis, penerbit dan orang yang disebut dalam pesan dapat meminta koreksi, pembatasan akses sebuah pesan, atau penghapusannya. Permintaan dijawab dalam 30 hari."))
                @if !contact.is_empty() { " " (t("Write to ", "Tulis ke ")) a href=(format!("mailto:{contact}")) { (contact) } (t(", giving the message ID.", ", dengan menyebut ID pesannya.")) } }
            p.small { (tf(|| format!("Archive built October 2026 · {} messages · software: a single Rust program (ingest, analysis, web server) with SQLite full-text search.", fmt_n(i(&o["messages"]))), || format!("Arsip dibangun Oktober 2026 · {} pesan · perangkat lunak: satu program Rust (pengolahan, analisis, server web) dengan pencarian teks lengkap SQLite.", fmt_n(i(&o["messages"]))))) }
        }
    };
    page(t("About", "Tentang"), user.as_ref(), "about", body)
}

// ---------------------------------------------------------------- login / register

#[derive(Deserialize, Default)]
pub struct NextQ {
    #[serde(default)]
    next: String,
}

fn safe_next(n: &str) -> String {
    if n.starts_with('/') && !n.starts_with("//") && !n.starts_with("/\\") { n.to_string() } else { "/read".into() }
}

fn login_page(user: Option<&auth::User>, next: &str, msg: Option<&str>) -> Markup {
    let body = html! {
        div.essay style="max-width:440px" {
            h1 { (t("Log in", "Masuk")) }
            p { (t("The reading room holds the full texts. Accounts are free for research.", "Ruang baca memuat teks lengkap. Akun gratis untuk penelitian.")) }
            @if let Some(m) = msg { div.flash { (m) } }
            form.stack method="post" action="/login" {
                (csrf_field())
                input type="hidden" name="next" value=(next);
                label for="email" { "Email" } input type="email" id="email" name="email" required autocomplete="username";
                label for="password" { (t("Password", "Kata sandi")) } input type="password" id="password" name="password" required autocomplete="current-password";
                p { button.btn type="submit" { (t("Log in", "Masuk")) } }
            }
            p.small { (t("No account? ", "Belum punya akun? ")) a href=(format!("/register?next={}", urlencoding::encode(next))) { (t("Register", "Daftar")) } ". " (t("Forgot your password? Ask an editor for a reset link.", "Lupa kata sandi? Mintalah tautan pengaturan ulang kepada editor.")) }
        }
    };
    page(t("Log in", "Masuk"), user, "login", body)
}

pub async fn login_form(MaybeUser(user): MaybeUser, Query(q): Query<NextQ>) -> Markup {
    login_page(user.as_ref(), &safe_next(&q.next), None)
}

#[derive(Deserialize)]
pub struct LoginForm {
    email: String,
    password: String,
    #[serde(default)]
    next: String,
}

pub async fn login(State(st): State<AppState>, ClientIp(ip): ClientIp, jar: CookieJar, Form(f): Form<LoginForm>) -> Result<Response, AppError> {
    let email = f.email.trim().to_lowercase();
    let (kip, kacct) = (format!("login-ip:{ip}"), format!("login-acct:{email}"));
    if st.throttle.recent(&kip, security::LOGIN_WINDOW) >= security::LOGIN_PER_IP || st.throttle.recent(&kacct, security::LOGIN_WINDOW) >= security::LOGIN_PER_ACCOUNT {
        return Ok(login_page(None, &safe_next(&f.next), Some(t("Too many attempts. Please wait 15 minutes and try again.", "Terlalu banyak percobaan. Tunggu 15 menit lalu coba lagi."))).into_response());
    }
    let pw = f.password.clone();
    let e2 = email.clone();
    let acct = site(&st, move |c| c.account_by_email(&e2)).await?;
    let verdict = tokio::task::spawn_blocking(move || match acct {
        Some(a) if auth::verify_password(&pw, &a.pw_hash) => Some(a),
        Some(_) => None,
        None => {
            auth::verify_dummy(&pw);
            None
        }
    })
    .await
    .map_err(|e| AppError(e.to_string()))?;
    match verdict {
        Some(a) if !a.approved => Ok(login_page(None, &safe_next(&f.next), Some(t("Your account is waiting for an editor's approval.", "Akun Anda sedang menunggu persetujuan editor."))).into_response()),
        Some(a) => {
            st.throttle.clear(&kacct);
            let id = a.id;
            let token = site(&st, move |c| c.create_session(id)).await?;
            Ok((jar.add(auth::session_cookie(token, st.secure_cookies)), Redirect::to(&safe_next(&f.next))).into_response())
        }
        None => {
            st.throttle.note(&kip);
            st.throttle.note(&kacct);
            Ok(login_page(None, &safe_next(&f.next), Some(t("That email and password do not match.", "Email dan kata sandi itu tidak cocok."))).into_response())
        }
    }
}

/// Version of the terms of use that new readers accept (recorded with each account).
pub const TERMS_VERSION: &str = "2026-10";

pub async fn logout(State(st): State<AppState>, jar: CookieJar) -> Result<Response, AppError> {
    if let Some(c) = jar.get(auth::COOKIE) {
        let tk = c.value().to_string();
        site(&st, move |c| c.delete_session(&tk)).await?;
    }
    Ok((jar.remove(Cookie::from(auth::COOKIE)), Redirect::to("/")).into_response())
}

fn register_page(user: Option<&auth::User>, next: &str, msg: Option<(&str, bool)>, approval: bool) -> Markup {
    let body = html! {
        div.essay style="max-width:520px" {
            h1 { (t("Register for the reading room", "Daftar ke ruang baca")) }
            p { (t("Free for research and teaching. ", "Gratis untuk penelitian dan pengajaran. ")) @if approval { (t("An editor approves each new account.", "Editor menyetujui setiap akun baru.")) } }
            @if let Some((m, ok)) = msg { div class=(if ok { "flash ok" } else { "flash" }) { (m) } }
            form.stack method="post" action="/register" {
                (csrf_field())
                input type="hidden" name="next" value=(next);
                label for="name" { (t("Name", "Nama")) } input type="text" id="name" name="name" required maxlength="120";
                label for="email" { "Email" } input type="email" id="email" name="email" required autocomplete="username";
                label for="affiliation" { (t("Affiliation", "Afiliasi")) } input type="text" id="affiliation" name="affiliation" maxlength="200" placeholder=(t("University, organisation or independent", "Universitas, organisasi atau independen"));
                label for="purpose" { (t("What will you use the archive for?", "Untuk apa Anda akan memakai arsip ini?")) } textarea id="purpose" name="purpose" maxlength="1000" {}
                label for="password" { (tf(|| format!("Password (at least {} characters)", auth::MIN_PASSWORD), || format!("Kata sandi (paling sedikit {} karakter)", auth::MIN_PASSWORD))) } input type="password" id="password" name="password" required minlength=(auth::MIN_PASSWORD) autocomplete="new-password";
                label { input type="checkbox" name="terms" value="1" required; " " (t("I will use the texts for research or teaching, cite the archive, and not republish full texts. I have read the ", "Saya akan memakai teks untuk penelitian atau pengajaran, mengutip arsip ini, dan tidak menerbitkan ulang teks lengkap. Saya sudah membaca ")) a href="/about#takedown" { (t("terms", "ketentuannya")) } "." }
                p { button.btn type="submit" { (t("Create account", "Buat akun")) } }
            }
        }
    };
    page(t("Register", "Daftar"), user, "login", body)
}

pub async fn register_form(State(st): State<AppState>, MaybeUser(user): MaybeUser, Query(q): Query<NextQ>) -> Markup {
    register_page(user.as_ref(), &safe_next(&q.next), None, st.approval)
}

#[derive(Deserialize)]
pub struct RegForm {
    name: String,
    email: String,
    #[serde(default)]
    affiliation: String,
    #[serde(default)]
    purpose: String,
    password: String,
    #[serde(default)]
    terms: String,
    #[serde(default)]
    next: String,
}

pub async fn register(State(st): State<AppState>, ClientIp(ip): ClientIp, jar: CookieJar, Form(f): Form<RegForm>) -> Result<Response, AppError> {
    let next = safe_next(&f.next);
    let key = format!("register-ip:{ip}");
    if st.throttle.recent(&key, security::REGISTER_WINDOW) >= security::REGISTER_PER_IP {
        return Ok(register_page(None, &next, Some((t("Too many registrations from this address. Please try again later.", "Terlalu banyak pendaftaran dari alamat ini. Silakan coba lagi nanti."), false)), st.approval).into_response());
    }
    let email = f.email.trim().to_lowercase();
    if f.terms != "1" || f.password.chars().count() < auth::MIN_PASSWORD || !email.contains('@') || f.name.trim().is_empty() {
        let m = tf(|| format!("Please fill in your name and email, choose a password of at least {} characters, and accept the terms.", auth::MIN_PASSWORD), || format!("Isi nama dan email Anda, pilih kata sandi paling sedikit {} karakter, dan setujui ketentuannya.", auth::MIN_PASSWORD));
        return Ok(register_page(None, &next, Some((&m, false)), st.approval).into_response());
    }
    st.throttle.note(&key);
    let approval = st.approval;
    let pw = f.password.clone();
    let hash = tokio::task::spawn_blocking(move || auth::hash_password(&pw)).await.map_err(|e| AppError(e.to_string()))?;
    let res = site(&st, move |c| {
        if c.account_by_email(&email)?.is_some() {
            return Ok(None);
        }
        let id = c.create_account(&email, f.name.trim(), f.affiliation.trim(), f.purpose.trim(), &hash, "reader", !approval, Some(TERMS_VERSION))?;
        if approval { Ok(Some(None)) } else { Ok(Some(Some(c.create_session(id)?))) }
    })
    .await?;
    match res {
        None => Ok(register_page(None, &next, Some((t("An account with that email already exists. Log in instead.", "Akun dengan email itu sudah ada. Silakan masuk."), false)), st.approval).into_response()),
        Some(None) => Ok(register_page(None, &next, Some((t("Thank you. An editor will approve your account; you can then log in.", "Terima kasih. Editor akan menyetujui akun Anda; setelah itu Anda dapat masuk."), true)), st.approval).into_response()),
        Some(Some(token)) => Ok((jar.add(auth::session_cookie(token, st.secure_cookies)), Redirect::to(&next)).into_response()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn essay_rendering() {
        let mut v = HashMap::new();
        v.insert("total", "137,146".to_string());
        let mut b = HashMap::new();
        b.insert("stats", html! { div.stats { "S" } });
        let (title, h) = render_essay("<!-- a comment -->\n# The title\n\n<p class=\"lede\">Lede {{total}}.</p>\n\n{{stats}}\n\nText {{unknown}}.\n", &v, &b);
        assert_eq!(title, "The title");
        assert!(h.contains("Lede 137,146.") && h.contains("<div class=\"stats\">S</div>") && !h.contains("comment"));
        assert!(h.contains("{{unknown}}"), "unknown names stay visible to the editor");
        assert!(!h.contains("<p><div"), "blocks replace their paragraph");
    }
    #[test]
    fn both_essays_use_only_known_names() {
        let known = ["total", "basis", "est", "coverage_share", "lost", "days_listed", "days_nofolder", "nineties_captured", "nineties_estimate", "list_messages", "duplicates", "canonical", "share_coded", "coverage_months_sentence", "english_1991", "indonesian_1994", "indonesian_1995", "indonesian_1996", "underground_1997_avg", "underground_peak", "underground_peak_quarter", "underground_peak_news", "model_pairs", "et_1992", "et_1999", "economy_1996", "economy_1998", "corruption_1995", "corruption_1998", "communal_1998", "communal_1999", "aceh_1998", "aceh_1999", "military_mention_lo", "military_mention_hi", "military_about_lo", "military_about_hi", "events_n", "lag_santa_cruz", "lag_27_july", "after30_27_july", "after30_referendum", "lag_marsinah", "days_1993_with_messages", "days_1993", "marsinah_survival", "may_paragraph", "may_focused", "may_busiest_month", "may_busiest_n", "gerwani_total", "stats", "figure:monthly", "figure:underground", "figure:themes", "figure:people", "read"];
        let re = regex::Regex::new(r"\{\{([a-z0-9_:]+)\}\}").unwrap();
        for src in [HOME_EN, HOME_ID] {
            let body = &src[src.find("-->").unwrap()..];
            for c in re.captures_iter(body) {
                assert!(known.contains(&&c[1]), "unknown name {{{{{}}}}}", &c[1]);
            }
        }
    }
}
