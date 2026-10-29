//! The privacy review: every message that speaks of the sexual violence of May 1998 (tiers 2
//! and 3) stays hidden from ordinary readers until an editor has looked at it. The editor
//! clears it for readers or keeps it restricted to researchers; a decision covers every copy of
//! the text. A first-pass suggestion (restrict or show, with a reason) is shown beside each
//! message to speed this up; only the editor's decision counts.

use super::admin::{excerpt, highlight_terms};
use super::auth::Admin;
use super::layout::{self, csrf_field, fmt_n, nice_date};
use super::{blocking, reload_restricted, site, AppError, AppState};
use crate::flags;
use axum::{
    extract::{Form, Query, State},
    response::Redirect,
};
use maud::{html, Markup};
use rusqlite::params_from_iter;
use serde::Deserialize;
use std::collections::HashMap;

const PER_PAGE: usize = 25;

#[derive(Deserialize, Default)]
pub struct ReviewQ {
    #[serde(default)]
    show: String,
    #[serde(default)]
    tier: String,
    page: Option<usize>,
}

struct Item {
    ak: String,
    date: Option<String>,
    title: String,
    tier: i64,
    copies: Vec<String>,
    body: String,
}

/// Originals of the May 1998 tier 2-3 messages, with the AK IDs of their copies.
fn items(c: &rusqlite::Connection) -> rusqlite::Result<Vec<Item>> {
    let rows: Vec<(i64, String, Option<String>, String, i64, Option<i64>)> = c
        .prepare("SELECT id, ak_id, date, title, may98_tier, dup_of FROM messages WHERE may98_tier >= 2 ORDER BY may98_tier DESC, date, id")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut copies: HashMap<i64, Vec<String>> = HashMap::new();
    for r in &rows {
        if let Some(root) = r.5 {
            copies.entry(root).or_default().push(r.1.clone());
        }
    }
    // a copy whose original is not in the tiers is reviewed on its own
    let ids: std::collections::HashSet<i64> = rows.iter().map(|r| r.0).collect();
    let mut out = Vec::new();
    let mut q = c.prepare("SELECT body FROM messages WHERE id = ?1")?;
    for (id, ak, date, title, tier, dup) in rows {
        if dup.map(|d| ids.contains(&d)).unwrap_or(false) {
            continue;
        }
        let body: String = q.query_row([id], |r| r.get(0))?;
        out.push(Item { ak, date, title, tier, copies: copies.remove(&id).unwrap_or_default(), body });
    }
    Ok(out)
}

pub async fn page(State(st): State<AppState>, Admin(u): Admin, Query(q): Query<ReviewQ>) -> Result<Markup, AppError> {
    let all = blocking(&st, items).await?;
    let (decisions, suggestions) = site(&st, |c| Ok((c.review_decisions()?, c.review_suggestions()?))).await?;
    let status = |it: &Item| decisions.get(&it.ak).map(|d| d.0.clone()).unwrap_or_else(|| "pending".into());
    let show = if q.show.is_empty() { "pending".to_string() } else { q.show.clone() };
    let counts = |s: &str| all.iter().filter(|it| status(it) == s).count();
    let (n_pending, n_cleared, n_restricted) = (counts("pending"), counts("cleared"), counts("restricted"));
    let n_sugg_restrict = all.iter().filter(|it| status(it) == "pending" && suggestions.get(&it.ak).map(|s| s.0 == "restrict").unwrap_or(false)).count();
    let filtered: Vec<&Item> = all
        .iter()
        .filter(|it| show == "all" || status(it) == show)
        .filter(|it| match q.tier.as_str() {
            "3" => it.tier == 3,
            "2" => it.tier == 2,
            _ => true,
        })
        .collect();
    let pages = filtered.len().div_ceil(PER_PAGE).max(1);
    let page_n = q.page.unwrap_or(1).clamp(1, pages);
    let shown: Vec<&&Item> = filtered.iter().skip((page_n - 1) * PER_PAGE).take(PER_PAGE).collect();
    let here = format!("/admin/review?show={show}&tier={}&page={page_n}", q.tier);
    let re = highlight_terms("may98:2");
    let tab = |key: &str, label: &str, n: usize| {
        html! { @if show == key { b { (label) " (" (fmt_n(n as i64)) ")" } } @else { a href=(format!("/admin/review?show={key}&tier={}", q.tier)) { (label) " (" (fmt_n(n as i64)) ")" } } }
    };
    let body = html! {
        div.wide {
            div.kicker { a href="/admin" { "Editors" } " / privacy review" }
            h1 { "Privacy review: May 1998" }
            p.lede style="max-width:780px" { "Messages that speak of the sexual violence of May 1998 stay hidden from ordinary readers until you decide. Restrict a message when it names or identifies a survivor, a witness or a private person (name, family, address, school, workplace); clear it when it reports, argues or denies without identifying anyone. A decision covers every copy of the text. Researchers see all of them either way." }
            p.small { "A first-pass suggestion is shown beside each message, made from the excerpts below; " (fmt_n(n_sugg_restrict as i64)) " pending messages are suggested for restriction. Read before you decide: the suggestion is an aid, and it may miss a name outside the excerpt." }
            p { (tab("pending", "Waiting", n_pending)) " · " (tab("restricted", "Restricted", n_restricted)) " · " (tab("cleared", "Cleared", n_cleared)) " · " (tab("all", "All", all.len()))
                " — " span.small {
                    @for (k, l) in [("", "both tiers"), ("3", "tier 3 only"), ("2", "tier 2 only")] {
                        @if q.tier == k { b { (l) } } @else { a href=(format!("/admin/review?show={show}&tier={k}")) { (l) } } " "
                    }
                }
            }
            form method="post" action="/admin/review" {
                (csrf_field())
                input type="hidden" name="back" value=(here);
                div style="display:flex;gap:8px;flex-wrap:wrap;position:sticky;top:0;background:var(--bg);padding:8px 0;z-index:2" {
                    button.btn type="submit" name="action" value="clear" { "Clear the checked (show to readers)" }
                    button.btn.ghost type="submit" name="action" value="restrict" { "Restrict the checked" }
                    button.btn.ghost type="submit" name="action" value="suggested" title="Restrict where the first pass suggests it, clear the others" { "Apply the suggestion to the checked" }
                    button.btn.ghost type="submit" name="action" value="pending" { "Undo: back to waiting" }
                }
                @if shown.is_empty() { p { "Nothing here." } }
                @for it in &shown {
                    @let st0 = status(it);
                    @let sg = suggestions.get(&it.ak);
                    div.card id=(it.ak) style="margin:14px 0" {
                        div style="display:flex;gap:10px;align-items:baseline;flex-wrap:wrap" {
                            input type="checkbox" name="ak" value=(it.ak) id=(format!("c-{}", it.ak));
                            label for=(format!("c-{}", it.ak)) style="font:600 17px/1.35 var(--serif)" { (it.title) }
                        }
                        div.meta.small {
                            a href=(format!("/read/m/{}", it.ak)) target="_blank" { (it.ak) } " · " (it.date.as_deref().map(nice_date).unwrap_or_default()) " · tier " (it.tier)
                            @if !it.copies.is_empty() { " · " (it.copies.len()) " " (if it.copies.len() == 1 { "copy" } else { "copies" }) }
                            " · now: " b { (st0) }
                        }
                        @if let Some((s, reason)) = sg {
                            p.small style="margin:6px 0" { span class=(format!("sugg {s}")) { "Suggested: " (s) } " " (reason) }
                        }
                        (excerpt(&it.body, &re, Some(&*flags::SV), 5))
                    }
                }
            }
            @if pages > 1 {
                div.pager {
                    @if page_n > 1 { a href=(format!("/admin/review?show={show}&tier={}&page={}", q.tier, page_n - 1)) { "← Previous" } }
                    span.small { "Page " (page_n) " of " (pages) }
                    @if page_n < pages { a href=(format!("/admin/review?show={show}&tier={}&page={}", q.tier, page_n + 1)) { "Next →" } }
                }
            }
        }
    };
    Ok(layout::page("Privacy review", Some(&u), "admin", body))
}

pub async fn decide(State(st): State<AppState>, Admin(u): Admin, Form(f): Form<Vec<(String, String)>>) -> Result<Redirect, AppError> {
    let get = |k: &str| f.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let action = get("action");
    let back = get("back");
    let back = if back.starts_with("/admin/review") { back } else { "/admin/review".to_string() };
    let picked: Vec<String> = f.iter().filter(|(k, _)| k == "ak").map(|(_, v)| v.clone()).filter(|v| v.starts_with("AK")).collect();
    if picked.is_empty() || !["clear", "restrict", "pending", "suggested"].contains(&action.as_str()) {
        return Ok(Redirect::to(&back));
    }
    // every copy of each picked text
    let p2 = picked.clone();
    let groups: Vec<(String, Vec<String>)> = blocking(&st, move |c| {
        let mut out = Vec::new();
        let mut q = c.prepare("SELECT m.ak_id FROM messages m WHERE m.id = (SELECT coalesce(dup_of, id) FROM messages WHERE ak_id = ?1) OR m.dup_of = (SELECT coalesce(dup_of, id) FROM messages WHERE ak_id = ?1)")?;
        for a in &p2 {
            let mut all: Vec<String> = q.query_map(params_from_iter([a]), |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            if !all.contains(a) {
                all.push(a.clone());
            }
            out.push((a.clone(), all));
        }
        Ok(out)
    })
    .await?;
    let uid = u.id;
    let n = picked.len();
    site(&st, move |c| {
        let sugg = if action == "suggested" { c.review_suggestions()? } else { HashMap::new() };
        for (ak, all) in &groups {
            let status = match action.as_str() {
                "clear" => "cleared",
                "restrict" => "restricted",
                "pending" => "pending",
                _ => match sugg.get(ak).map(|s| s.0.as_str()) {
                    Some("restrict") => "restricted",
                    Some("show") => "cleared",
                    _ => continue,
                },
            };
            c.set_review(all, status, uid)?;
        }
        c.audit(uid, "privacy review", &format!("{action}: {n} messages"))
    })
    .await?;
    reload_restricted(&st).await?;
    Ok(Redirect::to(&back))
}
