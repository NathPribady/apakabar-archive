//! Editors: approve readers, set roles, make reset links, mark sensitive messages, review
//! precision checks. (The editors' pages are in English.)

use super::auth::Admin;
use super::layout::{csrf_field, fmt_n, nice_date, page, pct};
use super::{blocking, reload_restricted, site, AppError, AppState};
use crate::{analyze, flags, lexicon};
use axum::{
    extract::{Form, Path, State},
    response::Redirect,
};
use maud::{html, Markup, PreEscaped};
use regex::Regex;
use rusqlite::OptionalExtension;
use serde::Deserialize;
use std::collections::HashMap;

/// Hours a password-reset link stays valid.
const RESET_HOURS: i64 = 72;

pub async fn home(State(st): State<AppState>, Admin(u): Admin) -> Result<Markup, AppError> {
    let since = (chrono::Utc::now() - chrono::Duration::days(30)).format("%Y-%m-%d").to_string();
    let (users, sens, reads, log) = site(&st, move |c| Ok((c.list_accounts()?, c.sensitive()?, c.reads_since(&since)?, c.recent_audit(30)?))).await?;
    let ids: Vec<String> = sens.iter().map(|s| s.0.clone()).collect();
    let titles: HashMap<String, String> = blocking(&st, move |c| {
        let mut out = HashMap::new();
        let mut q = c.prepare("SELECT title FROM messages WHERE ak_id = ?1")?;
        for a in &ids {
            if let Some(tt) = q.query_row([a], |r| r.get::<_, String>(0)).optional()? {
                out.insert(a.clone(), tt);
            }
        }
        Ok(out)
    })
    .await?;
    let pending = st.restricted.read().unwrap().pending.len();
    let waiting = users.iter().filter(|x| !x.approved).count();
    let body = html! {
        div.wide {
            div.kicker { "Editors" }
            h1 { "Readers and sensitive messages" }
            p { (users.len()) " accounts · " (waiting) " waiting for approval · " (fmt_n(reads)) " message views in the last 30 days · new readers " (if st.approval { "wait for approval" } else { "are approved automatically" }) "." }
            p { a href="/admin/review" { "Privacy review →" } " (" (fmt_n(pending as i64)) " May 1998 messages still hidden from readers) · " a href="/admin/checks" { "Precision checks →" } }
            p.small { "To help a reader who forgot their password, make a reset link and send it to them yourself; it works once, for " (RESET_HOURS) " hours. Blocking an account ends its sessions." }
            table.t {
                thead { tr { th { "Reader" } th { "Affiliation and purpose" } th { "Joined / last seen" } th { "Access" } th {} } }
                tbody {
                    @for a in &users {
                        tr {
                            td { b { (a.name) } br; (a.email) }
                            td { (a.affiliation) br; span.small { (a.purpose) } }
                            td.small { (a.created_at) br; (a.last_login.clone().unwrap_or_default()) }
                            td {
                                form method="post" action=(format!("/admin/user/{}", a.id)) style="display:flex;gap:6px;flex-wrap:wrap" {
                                    (csrf_field())
                                    select name="role" { @for r in ["reader", "researcher", "admin"] { option value=(r) selected[a.role == r] { (r) } } }
                                    select name="approved" { option value="1" selected[a.approved] { "approved" } option value="0" selected[!a.approved] { "blocked / waiting" } }
                                    button.btn.ghost type="submit" { "Save" }
                                }
                            }
                            td { form method="post" action=(format!("/admin/reset/{}", a.id)) { (csrf_field()) button.btn.ghost type="submit" title="Make a one-time link to set a new password" { "Reset link" } } }
                        }
                    }
                }
            }
            p.small { "Roles: a reader opens every message except restricted ones; a researcher also opens those; an admin is an editor." }
            h2 { "Restricted messages (" (sens.len()) ")" }
            p.small { "Only researchers and editors can open these. Mark a message from its page or on the privacy review page. Flags are kept in the site store by message ID, so they survive a rebuild." }
            ul { @for (a, at) in &sens { li { a href=(format!("/read/m/{a}")) { (a) } " " (titles.get(a).cloned().unwrap_or_else(|| "(not in the current database)".into())) " " span.small { (at) } } } }
            h2 { "Recent editorial actions" }
            table.t { tbody { @for (at, who, action, detail) in &log { tr { td.small { (at) } td { (who) } td { (action) } td.small { (detail) } } } } }
            p.small { "Site store: " (st.site.describe()) }
        }
    };
    let _ = u;
    Ok(page("Editors", Some(&u), "admin", body))
}

#[derive(Deserialize)]
pub struct UserForm {
    role: String,
    approved: i64,
}

pub async fn update_user(State(st): State<AppState>, Admin(u): Admin, Path(id): Path<i64>, Form(f): Form<UserForm>) -> Result<Redirect, AppError> {
    // an editor cannot lock themselves out (the site would be left without one)
    if !["reader", "researcher", "admin"].contains(&f.role.as_str()) || (id == u.id && (f.role != "admin" || f.approved != 1)) {
        return Ok(Redirect::to("/admin"));
    }
    site(&st, move |c| {
        c.update_account(id, &f.role, f.approved == 1)?;
        c.audit(u.id, "account", &format!("user {id}: role {}, approved {}", f.role, f.approved.clamp(0, 1)))
    })
    .await?;
    Ok(Redirect::to("/admin"))
}

pub async fn reset_link(State(st): State<AppState>, Admin(u): Admin, Path(id): Path<i64>) -> Result<Markup, AppError> {
    let uid = u.id;
    let (who, token) = site(&st, move |c| {
        let a = c.account_by_id(id)?;
        let t = c.create_reset(id, uid, RESET_HOURS)?;
        c.audit(uid, "reset link", &format!("user {id}"))?;
        Ok((a.map(|a| a.email).unwrap_or_default(), t))
    })
    .await?;
    let link = st.url(&format!("/reset/{token}"));
    let body = html! {
        div.essay {
            div.kicker { a href="/admin" { "Editors" } " / reset link" }
            h1 { "Reset link for " (who) }
            p { "Send this link to the reader yourself (by email or chat). It works once and expires in " (RESET_HOURS) " hours. It is shown only now." }
            div.cite { (link) }
            p { a href="/admin" { "← Back to the readers" } }
        }
    };
    Ok(page("Reset link", Some(&u), "admin", body))
}

/// Mark or unmark a text as sensitive: the message and every copy of it.
pub async fn toggle_sensitive(State(st): State<AppState>, Admin(u): Admin, Path(ak_id): Path<String>) -> Result<Redirect, AppError> {
    if !crate::web::read::plain_id(&ak_id) {
        return Ok(Redirect::to("/admin"));
    }
    let a = ak_id.clone();
    let group: Vec<String> = blocking(&st, move |c| crate::web::copies_of(c, &a)).await?;
    let on = {
        let r = st.restricted.read().unwrap();
        !group.iter().any(|g| r.sensitive.contains(g))
    };
    let a = ak_id.clone();
    site(&st, move |c| {
        c.set_sensitive(&group, u.id, on, "")?;
        c.audit(u.id, if on { "mark sensitive" } else { "unmark sensitive" }, &format!("{a} ({} copies)", group.len()))
    })
    .await?;
    reload_restricted(&st).await?;
    Ok(Redirect::to(&format!("/read/m/{ak_id}")))
}

// ---------------------------------------------------------------- precision checks

pub async fn checks(State(st): State<AppState>, Admin(u): Admin) -> Result<Markup, AppError> {
    let mut sets = st.checks()["sets"].as_array().cloned().unwrap_or_default();
    // each set followed by its earlier rounds, in the order the sets were first defined
    let base = |v: &serde_json::Value| analyze::check_base(v["key"].as_str().unwrap_or("")).to_string();
    let order: Vec<String> = sets.iter().map(base).collect();
    sets.sort_by_key(|v| (order.iter().position(|b| *b == base(v)).unwrap_or(usize::MAX), std::cmp::Reverse(v["round"].as_i64().unwrap_or(1))));
    let me = format!("user:{}", u.id);
    let counts = |s: &serde_json::Value, who: &str| -> Option<(i64, i64, i64)> {
        let r = s["results"].get(who)?;
        Some((r["yes"].as_i64().unwrap_or(0), r["no"].as_i64().unwrap_or(0), r["unsure"].as_i64().unwrap_or(0)))
    };
    let body = html! {
        div.wide {
            div.kicker { a href="/admin" { "Editors" } " / precision checks" }
            h1 { "Precision checks" }
            p.lede style="max-width:760px" { "Each theme, flag and name list is checked on 50 messages drawn at random (the same 50 every time). For each message, answer the question shown. Precision is the share of yes among yes and no answers. The goal before publishing a figure is 90%." }
            p.small style="max-width:760px" { "Round 1 was read before the Phase 1 changes (October 2026); round 2 drew fresh samples from what the site now shows, and round 3 read again the sets still below 90% after they were changed. Where a set was checked again, the site quotes the latest round." }
            p.small { "\"First pass\" is a preliminary reading made to speed up review; your own verdicts are what count." }
            table.t {
                thead { tr { th { "Set" } th.n { "In set" } th.n { "First pass" } th.n { "Your review" } th {} } }
                tbody {
                    @for s in &sets {
                        @let key = s["key"].as_str().unwrap_or("");
                        @let fp = counts(s, crate::site::FIRST_PASS);
                        @let mine = counts(s, &me);
                        tr {
                            td { b { (s["label"].as_str().unwrap_or("")) } br; span.small { (s["group"].as_str().unwrap_or("")) " · round " (s["round"].as_i64().unwrap_or(1)) @if s["current"].as_bool() == Some(false) { " (since checked again)" } } }
                            td.n { (fmt_n(s["total"].as_i64().unwrap_or(0))) }
                            td.n { @match fp { Some((y, n, un)) if y + n > 0 => { (pct(y as f64 / (y + n) as f64)) br; span.small { (y + n + un) " read" } } _ => { "–" } } }
                            td.n { @match mine { Some((y, n, un)) if y + n > 0 => { b { (pct(y as f64 / (y + n) as f64)) } br; span.small { (y + n + un) " of 50" } } _ => { "not started" } } }
                            td { a href=(format!("/admin/checks/{}", urlencoding::encode(key))) { "Review" } }
                        }
                    }
                }
            }
        }
    };
    Ok(page("Precision checks", Some(&u), "admin", body))
}

pub fn highlight_terms(key: &str) -> Option<Regex> {
    let key = analyze::check_base(key);
    let mut terms: Vec<String> = Vec::new();
    if let Some(slug) = key.strip_prefix("theme:").or_else(|| key.strip_prefix("about:")) {
        if let Some(t) = lexicon::THEMES.iter().find(|t| t.slug == slug) {
            terms.extend(t.terms.iter().chain(t.weak.iter()).chain(t.context.iter()).map(|s| s.to_string()));
        }
    } else if let Some(slug) = key.strip_prefix("entity:") {
        if let Some(e) = lexicon::ENTITIES.iter().find(|e| e.slug == slug) {
            terms.extend(e.aliases.iter().map(|s| s.to_string()));
        }
    } else if key.starts_with("may98") {
        return Regex::new(r"(?i)perkosa\w*|pemerkosa\w*|\brape\w*|sexual \w+|kekerasan seksual|pelecehan seksual|kerusuhan|\briots?\b|tionghoa|\bmei 1998\b|\bmay 1998\b|tgpf|tim relawan|martadinata").ok();
    }
    if terms.is_empty() {
        return None;
    }
    terms.sort_by_key(|t| std::cmp::Reverse(t.len()));
    let alt = terms.iter().map(|t| regex::escape(t)).collect::<Vec<_>>().join("|");
    Regex::new(&format!(r"(?i)\b(?:{alt})\b")).ok()
}

/// The paragraphs that matter for a verdict: the first one, those that match `first` (for the
/// May 1998 sets, the paragraphs that speak of sexual violence), then those with matched terms.
pub fn excerpt(body: &str, re: &Option<Regex>, first: Option<&Regex>, max_paras: usize) -> Markup {
    let paras: Vec<&str> = body.split("\n\n").collect();
    let mut keep: Vec<usize> = vec![0];
    for r in first.into_iter().chain(re.as_ref()) {
        for (i, p) in paras.iter().enumerate() {
            if i > 0 && keep.len() < max_paras && !keep.contains(&i) && r.is_match(p) {
                keep.push(i);
            }
        }
    }
    keep.sort_unstable();
    if paras.len() > 1 && keep.len() == 1 {
        keep.push(1);
    }
    html! {
        @for (k, i) in keep.iter().enumerate() {
            @if k > 0 && *i > keep[k - 1] + 1 { p.small { "…" } }
            @let p = window(paras[*i], first.or(re.as_ref()), 900);
            @let esc = html! { (p) }.into_string();
            p.snip { (PreEscaped(match re { Some(re) => re.replace_all(&esc, "<mark>$0</mark>").into_owned(), None => esc })) }
        }
    }
}

/// At most `max` characters of a paragraph, around the first match when the paragraph is long
/// (so a term deep inside a long paragraph is not cut off).
fn window(p: &str, r: Option<&Regex>, max: usize) -> String {
    let n = p.chars().count();
    if n <= max {
        return p.to_string();
    }
    let at = r.and_then(|r| r.find(p)).map(|m| p[..m.start()].chars().count()).unwrap_or(0);
    let start = at.saturating_sub(max / 3).min(n - max);
    let mut out: String = p.chars().skip(start).take(max).collect();
    if start > 0 {
        out = format!("… {}", out.trim_start());
    }
    if start + max < n {
        out.push_str(" …");
    }
    out
}

pub async fn check_set(State(st): State<AppState>, Admin(u): Admin, Path(key): Path<String>) -> Result<Markup, AppError> {
    let set = st.checks()["sets"].as_array().cloned().unwrap_or_default().into_iter().find(|s| s["key"].as_str() == Some(key.as_str()));
    let set = match set { Some(s) => s, None => return Err(AppError("no such check set".into())) };
    let sample: Vec<String> = set["sample"].as_array().cloned().unwrap_or_default().iter().filter_map(|x| x.as_str().map(|y| y.to_string())).collect();
    let s2 = sample.clone();
    let msgs: Vec<(String, Option<String>, Option<String>, String, String, String)> = blocking(&st, move |c| {
        let mut out = Vec::new();
        let mut q = c.prepare("SELECT ak_id, date, source_code, title, coalesce(title_source,''), body FROM messages WHERE ak_id = ?1")?;
        for a in &s2 {
            if let Some(m) = q.query_row([a], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))).optional()? {
                out.push(m);
            }
        }
        Ok(out)
    })
    .await?;
    let (k2, me) = (key.clone(), format!("user:{}", u.id));
    let verdicts: HashMap<String, (Option<(String, String)>, Option<String>)> = site(&st, move |c| {
        let mut out = HashMap::new();
        for a in &sample {
            let fp = c.verdict(&k2, a, crate::site::FIRST_PASS)?;
            let mine = c.verdict(&k2, a, &me)?.map(|v| v.0);
            out.insert(a.clone(), (fp, mine));
        }
        Ok(out)
    })
    .await?;
    let re = highlight_terms(&key);
    let first: Option<&Regex> = if key.starts_with("may98") { Some(&*flags::SV) } else { None };
    let body = html! {
        div.essay style="max-width:900px" {
            div.kicker { a href="/admin" { "Editors" } " / " a href="/admin/checks" { "precision checks" } }
            h1 { (set["label"].as_str().unwrap_or("")) }
            p.lede { (set["question"].as_str().unwrap_or("")) }
            p.small { (fmt_n(set["total"].as_i64().unwrap_or(0))) " messages in the set; these 50 are drawn at random and stay the same. Matched words are highlighted. Answer \"unsure\" when the excerpt is not enough, and open the message." }
            form method="post" action=(format!("/admin/checks/{}", urlencoding::encode(&key))) {
                (csrf_field())
                @for (n, (ak, date, code, title, tsrc, body)) in msgs.iter().enumerate() {
                    @let (fp, mine) = verdicts.get(ak).cloned().unwrap_or((None, None));
                    div.card style="margin:14px 0" {
                        div.meta { (n + 1) ". " a href=(format!("/read/m/{ak}")) target="_blank" { (ak) } " · " (date.as_deref().map(nice_date).unwrap_or_default()) @if let Some(c) = code { " · " (c) } }
                        h3 style="margin:4px 0" { (title) @if !tsrc.is_empty() && tsrc != "subject" { " " span.small { "(title: " (tsrc.replace('_', " ")) ")" } } }
                        (excerpt(body, &re, first, 4))
                        div style="display:flex;gap:14px;align-items:center;flex-wrap:wrap;font:14px var(--sans)" {
                            @for (v, label) in [("yes", "Yes"), ("no", "No"), ("unsure", "Unsure")] {
                                label { input type="radio" name=(ak) value=(v) checked[mine.as_deref() == Some(v)]; " " (label) }
                            }
                            @if let Some((v, note)) = fp { span.small { "First pass: " b { (v) } @if !note.is_empty() { " — " (note) } } }
                        }
                    }
                }
                p { button.btn type="submit" { "Save my answers" } }
            }
        }
    };
    Ok(page("Precision check", Some(&u), "admin", body))
}

pub async fn save_check_set(State(st): State<AppState>, Admin(u): Admin, Path(key): Path<String>, Form(f): Form<HashMap<String, String>>) -> Result<Redirect, AppError> {
    let reviewer = format!("user:{}", u.id);
    let verdicts: Vec<(String, String)> = f.into_iter().filter(|(ak, v)| ak.starts_with("AK") && ["yes", "no", "unsure"].contains(&v.as_str())).collect();
    let (k2, uid) = (key.clone(), u.id);
    let base = st.checks();
    let fresh = site(&st, move |c| {
        let n = c.save_verdicts(&k2, &reviewer, &verdicts)?;
        c.audit(uid, "precision check", &format!("{k2}: {n} answers"))?;
        super::refresh_checks(&base, c)
    })
    .await?;
    *st.checks.write().unwrap() = fresh;
    Ok(Redirect::to(&format!("/admin/checks/{}", urlencoding::encode(&key))))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_keep_the_match() {
        let long = format!("{} perkosaan {}", "a ".repeat(800), "b ".repeat(800));
        let r = Regex::new("perkosaan").unwrap();
        let w = window(&long, Some(&r), 900);
        assert!(w.contains("perkosaan") && w.starts_with('…') && w.ends_with('…'));
        assert!(w.chars().count() <= 904);
        assert_eq!(window("short", Some(&r), 900), "short");
    }
}
