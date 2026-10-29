//! The analysis behind the public homepage. Every number shown on the site is computed
//! here and stored as JSON in the `analysis` table, so it can be checked and re-run.
//!
//! Counts use "canonical" messages (duplicates folded into their first copy) unless noted.

use crate::{db, ingest::anyhow_lite::Result, lexicon, model, parse, tagger, text};
use ahash::{AHashMap, AHashSet};
use rayon::prelude::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

fn put(conn: &Connection, key: &str, v: &Value) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO analysis(key, json, computed_at) VALUES (?1, ?2, datetime('now'))",
        params![key, v.to_string()],
    )?;
    Ok(())
}

pub fn run(db_path: &Path, site: Option<&Path>) -> Result<()> {
    let t0 = Instant::now();
    let mut conn = db::open_rw(db_path)?;
    // the precision checks read the local site store (site.db): analysis never needs the
    // network, so it also runs where DATABASE_URL points at a server that is not reachable
    let site = crate::site::SiteDb::open(None, &crate::site::site_path(db_path, site), Some(db_path))?;
    if let Some(t) = crate::site::tools_dir() {
        crate::site::import_first_pass(&site, &t)?;
    }
    tag(&mut conn)?;
    eprintln!("tagged themes and entities in {:.1?}", t0.elapsed());
    let cov = coverage(&conn)?;
    overview(&conn)?;
    monthly(&conn, &cov)?;
    sources(&conn)?;
    censorship(&conn)?;
    themes(&conn)?;
    eprintln!("counts done in {:.1?}", t0.elapsed());
    entities(&conn)?;
    events(&conn)?;
    may1998(&conn)?;
    left1965(&conn)?;
    topics(&conn)?;
    distinctive_terms(&conn)?;
    let ch = compute_checks(&conn, &site)?;
    put(&conn, "checks", &ch)?;
    eprintln!("analysis complete in {:.1?}", t0.elapsed());
    Ok(())
}

/// Tag every message with the themes it mentions and is about, and with the names it contains
/// (see `tagger`).
fn tag(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute_batch("DELETE FROM message_themes; DELETE FROM message_entities; DELETE FROM themes; DELETE FROM entities; DELETE FROM events;")?;
    for (i, t) in lexicon::THEMES.iter().enumerate() {
        tx.execute("INSERT INTO themes(id, slug, name, description) VALUES (?1,?2,?3,?4)", params![i as i64 + 1, t.slug, t.name, t.description])?;
    }
    for (i, e) in lexicon::ENTITIES.iter().enumerate() {
        tx.execute("INSERT INTO entities(id, slug, name, kind) VALUES (?1,?2,?3,?4)", params![i as i64 + 1, e.slug, e.name, e.kind])?;
    }
    for (i, e) in lexicon::EVENTS.iter().enumerate() {
        tx.execute("INSERT INTO events(id, slug, date, name, description, query) VALUES (?1,?2,?3,?4,?5,?6)", params![i as i64 + 1, e.slug, e.date, e.name, e.description, e.query])?;
    }
    let tagger = tagger::Tagger::new();
    let rows: Vec<(i64, String, String, Option<String>)> = {
        let mut st = tx.prepare("SELECT id, title, body, date FROM messages")?;
        let it = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        it.collect::<rusqlite::Result<_>>()?
    };
    let tagged: Vec<(i64, Vec<(usize, u32, f64, bool)>, Vec<(usize, u32)>)> = rows
        .par_iter()
        .map(|(id, title, body, date)| {
            let (th, en) = tagger.tag(title, body);
            let digest = tagger::is_digest(title);
            let n = th.len();
            let themes = th
                .iter()
                .map(|h| {
                    let sc = model::score(&h.features(n, digest));
                    let slug = lexicon::THEMES[h.theme].slug;
                    (h.theme, h.hits(), sc, sc >= model::threshold(slug) && lexicon::theme_possible(slug, date.as_deref()))
                })
                .collect();
            (*id, themes, en)
        })
        .collect();
    {
        let mut it = tx.prepare("INSERT INTO message_themes(msg_id, theme_id, hits, score, about) VALUES (?1,?2,?3,?4,?5)")?;
        let mut ie = tx.prepare("INSERT INTO message_entities(msg_id, entity_id, hits) VALUES (?1,?2,?3)")?;
        for (id, th, en) in &tagged {
            for (i, n, sc, about) in th {
                it.execute(params![id, *i as i64 + 1, *n as i64, (sc * 1000.0).round() / 1000.0, *about as i64])?;
            }
            for (i, n) in en {
                ie.execute(params![id, *i as i64 + 1, *n as i64])?;
            }
        }
    }
    tx.execute_batch("UPDATE entities SET n_msgs = (SELECT count(*) FROM message_entities me JOIN messages m ON m.id = me.msg_id WHERE me.entity_id = entities.id AND m.counted = 1);")?;
    tx.commit()?;
    Ok(())
}

/// Features of the given messages, one line per theme they mention (for fitting the model).
pub fn dump_features(db_path: &Path, ids: &Path, out: &Path) -> Result<()> {
    let conn = db::open_rw(db_path)?;
    let tagger = tagger::Tagger::new();
    let mut w = std::io::BufWriter::new(std::fs::File::create(out)?);
    use std::io::Write;
    for ak in std::fs::read_to_string(ids)?.lines().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        let r: Option<(String, String)> = conn.query_row("SELECT title, body FROM messages WHERE ak_id = ?1", [ak], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        if let Some((title, body)) = r {
            let (th, _) = tagger.tag(&title, &body);
            let digest = tagger::is_digest(&title);
            for h in &th {
                writeln!(w, "{}", json!({"ak_id": ak, "theme": lexicon::THEMES[h.theme].slug, "x": h.features(th.len(), digest)}))?;
            }
        }
    }
    Ok(())
}

fn pairs(conn: &Connection, sql: &str) -> Result<Vec<(String, i64)>> {
    let mut st = conn.prepare(sql)?;
    let v = st.query_map([], |r| Ok((r.get::<_, rusqlite::types::Value>(0)?, r.get::<_, i64>(1)?)))?
        .map(|x| x.map(|(k, n)| (match k { rusqlite::types::Value::Text(t) => t, rusqlite::types::Value::Integer(i) => i.to_string(), _ => String::new() }, n)))
        .collect::<rusqlite::Result<_>>()?;
    Ok(v)
}

fn scalar(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get::<_, Option<i64>>(0)).ok().flatten().unwrap_or(0)
}

fn overview(conn: &Connection) -> Result<()> {
    let langs: Vec<(String, i64)> = conn
        .prepare("SELECT lang, count(*) FROM messages WHERE counted = 1 GROUP BY lang ORDER BY 2 DESC")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let types: Vec<(String, i64)> = conn
        .prepare("SELECT msg_type, count(*) FROM messages WHERE counted = 1 GROUP BY msg_type ORDER BY 2 DESC")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let senders: Vec<(String, i64)> = conn
        .prepare("SELECT lower(CASE WHEN instr(sender,' (')>0 THEN substr(sender,1,instr(sender,' (')-1) ELSE sender END) s, count(*) FROM messages GROUP BY s ORDER BY 2 DESC LIMIT 12")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let v = json!({
        "messages": scalar(conn, "SELECT count(*) FROM messages"),
        "basisdata": scalar(conn, "SELECT count(*) FROM messages WHERE collection='basisdata'"),
        "list": scalar(conn, "SELECT count(*) FROM messages WHERE collection='list'"),
        "canonical": scalar(conn, "SELECT count(*) FROM messages WHERE counted = 1"),
        "duplicates": scalar(conn, "SELECT count(*) FROM messages WHERE dup_of IS NOT NULL"),
        "topic_replies": scalar(conn, "SELECT count(*) FROM messages WHERE is_topic_reply=1"),
        "iri_l": scalar(conn, "SELECT count(*) FROM messages WHERE collection='iri-l'"),
        "month_folder": scalar(conn, "SELECT count(*) FROM messages WHERE collection='basisdata' AND length(ak_id) = 15"),
        "html_fixed": scalar(conn, "SELECT count(*) FROM messages WHERE html_fixed > 0"),
        "undated": scalar(conn, "SELECT count(*) FROM messages WHERE date IS NULL"),
        "date_notes": scalar(conn, "SELECT count(*) FROM messages WHERE date_note IS NOT NULL"),
        "titles_by_source": pairs(conn, "SELECT coalesce(title_source,'subject'), count(*) FROM messages WHERE counted = 1 GROUP BY 1 ORDER BY 2 DESC")?,
        "may98_tiers": pairs(conn, "SELECT may98_tier, count(*) FROM messages WHERE counted = 1 AND may98_tier > 0 GROUP BY 1 ORDER BY 1")?,
        "threads": scalar(conn, "SELECT count(DISTINCT thread_id) FROM messages WHERE thread_id IS NOT NULL"),
        "in_threads": scalar(conn, "SELECT count(*) FROM messages WHERE thread_id IS NOT NULL"),
        "words": scalar(conn, "SELECT sum(words) FROM messages WHERE counted = 1"),
        "chars": scalar(conn, "SELECT sum(chars) FROM messages WHERE counted = 1"),
        "first_date": conn.query_row("SELECT min(date) FROM messages", [], |r| r.get::<_, Option<String>>(0))?,
        "last_date": conn.query_row("SELECT max(date) FROM messages", [], |r| r.get::<_, Option<String>>(0))?,
        "coded": scalar(conn, "SELECT count(*) FROM messages WHERE counted = 1 AND source_code IS NOT NULL"),
        "sv": scalar(conn, "SELECT count(*) FROM messages WHERE counted = 1 AND sv_flag=1"),
        "may98": scalar(conn, "SELECT count(*) FROM messages WHERE counted = 1 AND may98_flag=1"),
        "repaired": scalar(conn, "SELECT count(*) FROM messages WHERE repaired>0"),
        "languages": langs,
        "types": types,
        "senders": senders,
    });
    put(conn, "overview", &v)
}

fn monthly(conn: &Connection, cov: &Value) -> Result<()> {
    let mut m: BTreeMap<String, AHashMap<String, i64>> = BTreeMap::new();
    let mut st = conn.prepare("SELECT ym, CASE WHEN lang IN ('id','en','mixed') THEN lang ELSE 'other' END, count(*) FROM messages WHERE counted = 1 AND ym IS NOT NULL GROUP BY 1,2")?;
    for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))? {
        let (ym, l, n) = r?;
        *m.entry(ym).or_default().entry(l).or_default() += n;
    }
    // every month from Oct 1990 to Feb 2002, zero-filled, so gaps show
    let mut rows = Vec::new();
    let (mut y, mut mo) = (1990, 10);
    while (y, mo) <= (2002, 2) {
        let k = format!("{:04}-{:02}", y, mo);
        let e = m.get(&k);
        let g = |l: &str| e.and_then(|x| x.get(l)).copied().unwrap_or(0);
        let total = g("id") + g("en") + g("mixed") + g("other");
        rows.push(json!({"ym": k, "id": g("id"), "en": g("en"), "mixed": g("mixed"), "other": g("other"), "total": total}));
        mo += 1;
        if mo == 13 { mo = 1; y += 1; }
    }
    // thin months: less than half of the estimated messages survive (see coverage)
    let thin: Vec<Value> = cov["months"].as_array().cloned().unwrap_or_default().into_iter()
        .filter(|m| m["estimate"].as_f64().unwrap_or(0.0) >= 20.0 && m["share"].as_f64().unwrap_or(1.0) < 0.5)
        .map(|m| m["ym"].clone()).collect();
    // language share by year
    let mut years: BTreeMap<String, (i64, i64, i64)> = BTreeMap::new();
    for r in &rows {
        let y = r["ym"].as_str().unwrap()[..4].to_string();
        let e = years.entry(y).or_default();
        e.0 += r["id"].as_i64().unwrap();
        e.1 += r["en"].as_i64().unwrap();
        e.2 += r["total"].as_i64().unwrap();
    }
    let ys: Vec<Value> = years.iter().map(|(y, (i, e, t))| json!({"year": y, "id": i, "en": e, "total": t, "share_id": if *t > 0 { *i as f64 / *t as f64 } else { 0.0 }})).collect();
    put(conn, "monthly", &json!({"months": rows, "thin": thin, "years": ys}))
}

fn sources(conn: &Connection) -> Result<()> {
    let mut top = Vec::new();
    let mut st = conn.prepare("SELECT source_code, max(source_name), max(source_cat), count(*), min(year), max(year) FROM messages WHERE counted = 1 AND source_code IS NOT NULL GROUP BY source_code ORDER BY 4 DESC LIMIT 60")?;
    for r in st.query_map([], |r| Ok(json!({"code": r.get::<_, String>(0)?, "name": r.get::<_, Option<String>>(1)?, "category": r.get::<_, String>(2)?, "n": r.get::<_, i64>(3)?, "first": r.get::<_, Option<i64>>(4)?, "last": r.get::<_, Option<i64>>(5)?})))? {
        top.push(r?);
    }
    let mut cat_year: BTreeMap<i64, BTreeMap<String, i64>> = BTreeMap::new();
    let mut st = conn.prepare("SELECT year, source_cat, count(*) FROM messages WHERE counted = 1 AND year IS NOT NULL GROUP BY 1,2")?;
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))? {
        let (y, c, n) = r?;
        cat_year.entry(y).or_default().insert(c, n);
    }
    let cy: Vec<Value> = cat_year.iter().map(|(y, m)| json!({"year": y, "cats": m})).collect();
    // top 8 sources per year
    let mut per_year: BTreeMap<i64, Vec<Value>> = BTreeMap::new();
    let mut st = conn.prepare("SELECT year, source_code, count(*) n FROM messages WHERE counted = 1 AND source_code IS NOT NULL AND year IS NOT NULL GROUP BY 1,2 ORDER BY 1, 3 DESC")?;
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))? {
        let (y, c, n) = r?;
        let v = per_year.entry(y).or_default();
        if v.len() < 8 {
            v.push(json!({"code": c, "n": n}));
        }
    }
    put(conn, "sources", &json!({"top": top, "by_year_category": cy, "top_by_year": per_year}))
}

/// What share of reposted news came from Indonesian outlets, from foreign press and wires,
/// and from banned or underground outlets, quarter by quarter.
fn censorship(conn: &Connection) -> Result<()> {
    // underground outlets at any date, plus the online and print successors of the banned
    // weeklies (Tempo Interaktif, D&R) between the June 1994 bans and the end of licensing in 1998
    let mut q: BTreeMap<String, (i64, i64, i64, i64)> = BTreeMap::new(); // total news, indonesian, foreign+wire, banned/alt
    let mut st = conn.prepare("SELECT date, source_code, source_cat FROM messages WHERE counted = 1 AND msg_type='news' AND date IS NOT NULL")?;
    for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?)))? {
        let (d, code, cat) = r?;
        let y = &d[..4];
        let m: u32 = d[5..7].parse().unwrap_or(1);
        let key = format!("{}-Q{}", y, (m - 1) / 3 + 1);
        let e = q.entry(key).or_default();
        e.0 += 1;
        if cat.starts_with("id_") || cat == "alternative" { e.1 += 1; }
        if cat == "foreign" || cat == "wire" { e.2 += 1; }
        let succ = matches!(code.as_deref(), Some("TI") | Some("D&R"));
        let in_ban = d.as_str() >= "1994-06-21" && d.as_str() <= "1998-12-31";
        if cat == "alternative" || (succ && in_ban) { e.3 += 1; }
    }
    let rows: Vec<Value> = q.iter().map(|(k, (t, i, f, b))| json!({"q": k, "news": t, "indonesian": i, "foreign": f, "banned_alt": b})).collect();
    let mut by_code = Vec::new();
    let mut st = conn.prepare("SELECT source_code, year, count(*) FROM messages WHERE counted = 1 AND source_code IN ('TEMPO','EDITOR','DETIK','TI','D&R','SIAR','KDP','PIJAR','XPOS','PRD','AJI','INDEPENDEN','PEMBEBASAN') GROUP BY 1,2 ORDER BY 1,2")?;
    for r in st.query_map([], |r| Ok(json!({"code": r.get::<_, String>(0)?, "year": r.get::<_, Option<i64>>(1)?, "n": r.get::<_, i64>(2)?})))? {
        by_code.push(r?);
    }
    put(conn, "censorship", &json!({"quarters": rows, "banned_alt_codes": ["TEMPO", "EDITOR", "DETIK", "TI", "D&R", "SIAR", "XPOS", "KDP", "PIJAR", "PEMBEBASAN", "PRD", "AJI", "INDEPENDEN"], "by_code_year": by_code}))
}

fn themes(conn: &Connection) -> Result<()> {
    let mut totals: BTreeMap<String, i64> = BTreeMap::new();
    let mut st = conn.prepare("SELECT ym, count(*) FROM messages WHERE counted = 1 AND ym IS NOT NULL GROUP BY ym")?;
    for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (k, n) = r?;
        totals.insert(k, n);
    }
    let share = |n: i64, tot: i64| if tot > 0 { n as f64 / tot as f64 } else { 0.0 };
    let mut out = Vec::new();
    for (i, t) in lexicon::THEMES.iter().enumerate() {
        let tid = i as i64 + 1;
        // per month: messages about the theme (the model's judgement) and all that mention it
        let mut months: BTreeMap<String, (i64, i64)> = BTreeMap::new();
        let mut st = conn.prepare("SELECT m.ym, sum(mt.about), count(*) FROM message_themes mt JOIN messages m ON m.id = mt.msg_id WHERE mt.theme_id = ?1 AND m.counted = 1 AND m.ym IS NOT NULL GROUP BY m.ym")?;
        for r in st.query_map([tid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))? {
            let (k, a, n) = r?;
            months.insert(k, (a, n));
        }
        let mut yearly: BTreeMap<String, (i64, i64, i64)> = BTreeMap::new();
        for (ym, tot) in &totals {
            let e = yearly.entry(ym[..4].to_string()).or_default();
            let (a, n) = months.get(ym).copied().unwrap_or((0, 0));
            e.0 += a;
            e.1 += n;
            e.2 += tot;
        }
        let total: i64 = months.values().map(|v| v.0).sum();
        let mentions: i64 = months.values().map(|v| v.1).sum();
        let peak = months.iter().max_by_key(|(_, v)| v.0).map(|(k, v)| json!({"ym": k, "n": v.0}));
        let mut ex = Vec::new();
        let mut st = conn.prepare("SELECT m.ak_id, m.date, m.title, m.source_code FROM message_themes mt JOIN messages m ON m.id = mt.msg_id WHERE mt.theme_id = ?1 AND m.counted = 1 AND mt.about = 1 ORDER BY mt.score DESC, mt.hits DESC LIMIT 8")?;
        for r in st.query_map([tid], |r| Ok(json!({"ak_id": r.get::<_, String>(0)?, "date": r.get::<_, Option<String>>(1)?, "title": r.get::<_, String>(2)?, "source": r.get::<_, Option<String>>(3)?})))? {
            ex.push(r?);
        }
        out.push(json!({
            "slug": t.slug, "name": t.name, "description": t.description, "terms": t.terms, "weak": t.weak, "stops": t.stops, "context": t.context,
            "total": total, "mentions": mentions, "threshold": model::threshold(t.slug), "peak": peak,
            "yearly": yearly.iter().map(|(y, (a, n, tot))| json!({"year": y, "n": a, "mentions": n, "share": share(*a, *tot), "mention_share": share(*n, *tot)})).collect::<Vec<_>>(),
            "monthly": months.iter().map(|(k, (a, n))| { let tot = *totals.get(k).unwrap_or(&1); json!({"ym": k, "n": a, "mentions": n, "share": share(*a, tot), "mention_share": share(*n, tot)}) }).collect::<Vec<_>>(),
            "examples": ex,
        }));
    }
    put(conn, "themes", &json!({"themes": out}))
}

fn entities(conn: &Connection) -> Result<()> {
    let mut list = Vec::new();
    let mut st = conn.prepare("SELECT id, slug, name, kind, n_msgs FROM entities ORDER BY n_msgs DESC")?;
    let ents: Vec<(i64, String, String, String, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<rusqlite::Result<_>>()?;
    for (id, slug, name, kind, n) in &ents {
        let mut yearly = BTreeMap::new();
        let mut st = conn.prepare("SELECT m.year, count(*) FROM message_entities me JOIN messages m ON m.id = me.msg_id WHERE me.entity_id = ?1 AND m.counted = 1 GROUP BY m.year")?;
        for r in st.query_map([id], |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, i64>(1)?)))? {
            let (y, c) = r?;
            if let Some(y) = y { yearly.insert(y.to_string(), c); }
        }
        list.push(json!({"slug": slug, "name": name, "kind": kind, "n": n, "yearly": yearly}));
    }
    // co-occurrence of the 24 most-mentioned people and organisations, by period
    let top: Vec<i64> = ents.iter().filter(|e| e.3 != "place").take(24).map(|e| e.0).collect();
    let topset: AHashSet<i64> = top.iter().copied().collect();
    let mut per_msg: AHashMap<i64, (i64, Vec<i64>)> = AHashMap::new();
    let mut st = conn.prepare("SELECT me.msg_id, m.year, me.entity_id FROM message_entities me JOIN messages m ON m.id = me.msg_id WHERE m.counted = 1")?;
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, i64>(2)?)))? {
        let (mid, y, e) = r?;
        if topset.contains(&e) {
            let x = per_msg.entry(mid).or_insert((y.unwrap_or(0), vec![]));
            x.1.push(e);
        }
    }
    let period = |y: i64| if y < 1998 { "1990-1997" } else if y <= 1999 { "1998-1999" } else { "2000-2002" };
    let mut pairs: BTreeMap<&str, AHashMap<(i64, i64), i64>> = BTreeMap::new();
    for (_, (y, es)) in per_msg {
        let p = period(y);
        let mut es = es;
        es.sort();
        es.dedup();
        for a in 0..es.len() {
            for b in a + 1..es.len() {
                *pairs.entry(p).or_default().entry((es[a], es[b])).or_default() += 1;
            }
        }
    }
    let name_of: AHashMap<i64, String> = ents.iter().map(|e| (e.0, e.2.clone())).collect();
    let slug_of: AHashMap<i64, String> = ents.iter().map(|e| (e.0, e.1.clone())).collect();
    let mut co = serde_json::Map::new();
    for (p, m) in pairs {
        let mut v: Vec<((i64, i64), i64)> = m.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        co.insert(p.to_string(), json!(v.iter().take(40).map(|((a, b), n)| json!({"a": name_of[a], "a_slug": slug_of[a], "b": name_of[b], "b_slug": slug_of[b], "n": n})).collect::<Vec<_>>()));
    }
    put(conn, "entities", &json!({"entities": list, "cooccurrence": co}))
}

/// Days since 1970-01-01 back to a date.
fn civil_from_days(z: i64) -> String {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

fn median(v: &mut [u32]) -> u32 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[v.len() / 2]
}

/// How much of the database survives. The database filed its messages in a folder per day,
/// numbered from 0000. Where the Wayback Machine kept the folder's own listing (see
/// `listings`), the count of the day is exact; where a month's listing shows that a day had no
/// folder, nothing was filed that day. Elsewhere the highest number on a day shows the least
/// that was posted, and a day with no surviving message takes the median of the surrounding six
/// weeks. From 4 January to 11 March 2000 the database used one month folder instead, numbered
/// 0000 to 2571.
fn coverage(conn: &Connection) -> Result<Value> {
    let rows: Vec<(String, Option<String>)> = conn
        .prepare("SELECT ak_id, date FROM messages WHERE collection = 'basisdata'")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut day: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    let mut folder_days: BTreeMap<String, u32> = BTreeMap::new();
    let (mut folder_n, mut folder_max) = (0u32, 0u32);
    for (ak, d) in &rows {
        match parse::id_kind(ak) {
            parse::IdKind::Day => {
                if let (Some(d), Some(n)) = (parse::id_date(ak), parse::id_number(ak)) {
                    let e = day.entry(d).or_insert((0, 0));
                    e.0 += 1;
                    e.1 = e.1.max(n);
                }
            }
            parse::IdKind::Month => {
                folder_n += 1;
                folder_max = folder_max.max(parse::id_number(ak).unwrap_or(0));
                if let Some(d) = d {
                    *folder_days.entry(d.clone()).or_default() += 1;
                }
            }
            parse::IdKind::List => {}
        }
    }
    let listed: BTreeMap<String, u32> = conn
        .prepare("SELECT day, listed FROM day_listings")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32)))?
        .collect::<rusqlite::Result<_>>()?;
    let folders: BTreeMap<String, AHashSet<String>> = conn
        .prepare("SELECT ym, days FROM month_folders")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .map(|x| x.map(|(k, v)| (k, v.split(',').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect())))
        .collect::<rusqlite::Result<_>>()?;
    let lost_n: i64 = conn.query_row("SELECT count(*) FROM lost", [], |r| r.get(0))?;
    let lost_by_day: BTreeMap<String, u32> = conn
        .prepare("SELECT date, count(*) FROM lost GROUP BY date")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32)))?
        .collect::<rusqlite::Result<_>>()?;
    let first = day_index("1990-10-07");
    let last = day.keys().chain(folder_days.keys()).map(|d| day_index(d)).max().unwrap_or(first);
    // (date, captured, lower bound, status)
    let mut cal: Vec<(String, u32, u32, &'static str)> = Vec::new();
    for z in first..=last {
        let d = civil_from_days(z);
        let mut captured = 0;
        let mut lower = 0;
        let mut status = "none";
        if let Some(&(c, mx)) = day.get(&d) {
            captured = c;
            lower = mx + 1;
            status = if c == mx + 1 { "complete" } else { "partial" };
        }
        if let Some(&c) = folder_days.get(&d) {
            captured += c;
            lower += c;
            status = if status == "none" || status == "complete" { "folder" } else { status };
        }
        if listed.contains_key(&d) {
            // the day's folder listing names every message filed that day, so the day held what
            // survives and what the listing names but no capture holds. A number the database
            // skipped (0011 on 31 January 2001 is in neither) is not a lost message.
            let gone = lost_by_day.get(&d).copied().unwrap_or(0);
            lower = captured + gone;
            status = if gone == 0 { "complete" } else { "partial" };
        } else if status == "none" {
            if let Some(f) = folders.get(&d[..7]) {
                if !f.contains(&d[8..10]) {
                    status = "nofolder";
                }
            }
        }
        cal.push((d, captured, lower, status));
    }
    let known: Vec<(i64, u32)> = cal.iter().enumerate().filter(|(_, c)| c.3 != "none" && c.3 != "nofolder").map(|(i, c)| (i as i64, c.2)).collect();
    let estimate: Vec<u32> = cal
        .iter()
        .enumerate()
        .map(|(i, c)| {
            if c.3 != "none" {
                return c.2;
            }
            for w in [21i64, 60] {
                let mut v: Vec<u32> = known.iter().filter(|(k, _)| (k - i as i64).abs() <= w).map(|(_, n)| *n).collect();
                if v.len() >= 5 {
                    return median(&mut v);
                }
            }
            0
        })
        .collect();
    let mut months: BTreeMap<String, (u64, u64, u64, u32, u32, u32)> = BTreeMap::new(); // captured, lower, estimate, days, days_none, days_nofolder
    let mut years: BTreeMap<String, (u64, u64, u64, u32, u32, u32)> = BTreeMap::new();
    for (i, c) in cal.iter().enumerate() {
        for (map, key) in [(&mut months, c.0[..7].to_string()), (&mut years, c.0[..4].to_string())] {
            let e = map.entry(key).or_default();
            e.0 += c.1 as u64;
            e.1 += c.2 as u64;
            e.2 += estimate[i] as u64;
            e.3 += 1;
            if c.3 == "none" {
                e.4 += 1;
            }
            if c.3 == "nofolder" {
                e.5 += 1;
            }
        }
    }
    let row = |k: &String, v: &(u64, u64, u64, u32, u32, u32), key: &str| {
        json!({key: k, "captured": v.0, "lower_bound": v.1, "estimate": v.2, "share": if v.2 > 0 { v.0 as f64 / v.2 as f64 } else { 1.0 }, "days": v.3, "days_none": v.4, "days_nofolder": v.5})
    };
    let tot_c: u64 = cal.iter().map(|c| c.1 as u64).sum();
    let tot_l: u64 = cal.iter().map(|c| c.2 as u64).sum();
    let tot_e: u64 = estimate.iter().map(|n| *n as u64).sum();
    let nineties = years.iter().filter(|(y, _)| y.as_str() <= "1999").fold((0u64, 0u64), |a, (_, v)| (a.0 + v.0, a.1 + v.2));
    // longest runs of days with nothing captured
    let mut gaps: Vec<(String, String, usize)> = Vec::new();
    let mut k = 0;
    while k < cal.len() {
        if cal[k].3 == "none" {
            let s0 = k;
            while k < cal.len() && cal[k].3 == "none" {
                k += 1;
            }
            gaps.push((cal[s0].0.clone(), cal[k - 1].0.clone(), k - s0));
        } else {
            k += 1;
        }
    }
    gaps.sort_by(|a, b| b.2.cmp(&a.2));
    let count = |s: &str| cal.iter().filter(|c| c.3 == s).count();
    let v = json!({
        "captured": tot_c, "lower_bound": tot_l, "estimate": tot_e, "share": if tot_e > 0 { tot_c as f64 / tot_e as f64 } else { 0.0 },
        "missing_in_captured_days": tot_l - tot_c,
        "days": {"calendar": cal.len(), "complete": count("complete"), "partial": count("partial"), "folder": count("folder"), "none": count("none"), "nofolder": count("nofolder"), "listed": listed.len()},
        "lost": lost_n,
        "listed_messages": listed.values().map(|n| *n as u64).sum::<u64>() + folder_n as u64,
        "nineties": {"captured": nineties.0, "estimate": nineties.1, "macdougall": 100000, "macdougall_source": "AK-2000-05-13-0026"},
        "month_folder": {"messages": folder_n, "highest_number": folder_max, "from": folder_days.keys().next(), "to": folder_days.keys().last()},
        "years": years.iter().map(|(k, v)| row(k, v, "year")).collect::<Vec<_>>(),
        "months": months.iter().map(|(k, v)| row(k, v, "ym")).collect::<Vec<_>>(),
        "gaps": gaps.iter().take(12).map(|g| json!({"from": g.0, "to": g.1, "days": g.2})).collect::<Vec<_>>(),
    });
    put(conn, "coverage", &v)?;
    Ok(v)
}

/// The conference topics that replies ("(r)") were filed under.
fn topics(conn: &Connection) -> Result<()> {
    let mut list = Vec::new();
    let mut st = conn.prepare("SELECT topic, count(*), min(date), max(date) FROM messages WHERE counted = 1 AND topic IS NOT NULL GROUP BY topic ORDER BY 2 DESC")?;
    for r in st.query_map([], |r| Ok(json!({"name": r.get::<_, String>(0)?, "n": r.get::<_, i64>(1)?, "first": r.get::<_, Option<String>>(2)?, "last": r.get::<_, Option<String>>(3)?})))? {
        list.push(r?);
    }
    let n_msgs: i64 = list.iter().map(|t| t["n"].as_i64().unwrap_or(0)).sum();
    let big = list.iter().filter(|t| t["n"].as_i64().unwrap_or(0) >= 20).count();
    put(conn, "topics", &json!({"n_topics": list.len(), "n_messages": n_msgs, "n_with_20_or_more": big, "topics": list.into_iter().take(400).collect::<Vec<_>>()}))
}

pub struct CheckSet {
    pub key: String,
    pub label: String,
    pub group: &'static str,
    pub question: String,
    /// 1: the first reading (October 2026, before the Phase 1 changes); 2: fresh samples after
    /// them; 3: the sets below 90% in round 2, after the changes that followed
    pub round: u8,
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// The part of a check-set key that names what is checked: "may98:2/2" -> "may98:2".
pub fn check_base(key: &str) -> &str {
    key.split('/').next().unwrap_or(key)
}

/// The check sets: 50 messages drawn from each theme, flag and name list, with the question
/// a reviewer answers for each. The same messages are drawn every time (seeded order). A set
/// checked again after a change gets a new key ("entity:medan/2") and so a fresh sample; the
/// earlier round stays for the record.
pub fn check_sets() -> Vec<CheckSet> {
    let mut v = Vec::new();
    let may2 = "Does the message speak of the sexual violence of the May 1998 riots (reports, testimony, denials, inquiries)?";
    let may3 = "Is the message mainly about the sexual violence of the May 1998 riots?";
    let title_q = "Is the title a fair title for this message?";
    let titles = [("headline", "Titles taken from a headline line"), ("opening", "Titles taken from the opening words"), ("duplicate", "Titles taken from another copy"), ("body_prefix", "Cut subjects completed from the text")];
    // round 2: what the site now shows
    for t in lexicon::THEMES {
        v.push(CheckSet { key: format!("about:{}", t.slug), label: t.name.to_string(), group: "Themes: messages about each theme", question: format!("Is the message substantially about {}: {}?", t.name, lower_first(t.description)), round: 2 });
    }
    v.push(CheckSet { key: "may98:2/2".into(), label: "May 1998, tiers 2 and 3".into(), group: "May 1998 flag", question: may2.into(), round: 2 });
    v.push(CheckSet { key: "may98:3/2".into(), label: "May 1998, tier 3 (focused)".into(), group: "May 1998 flag", question: may3.into(), round: 2 });
    for slug in ["jakarta", "medan", "washington"] {
        if let Some(e) = lexicon::ENTITIES.iter().find(|e| e.slug == slug) {
            v.push(CheckSet { key: format!("entity:{slug}/2"), label: e.name.to_string(), group: "Names", question: format!("Does the message name {} (not an outlet, airport or namesake)?", e.name), round: 2 });
        }
    }
    for (src, label) in &titles[..2] {
        v.push(CheckSet { key: format!("title:{src}/2"), label: label.to_string(), group: "Titles", question: title_q.into(), round: 2 });
    }
    // round 3: the sets below 90% in round 2, checked again after the changes that followed
    for slug in ["aceh", "human-rights", "women", "may-1998"] {
        if let Some(t) = lexicon::THEMES.iter().find(|t| t.slug == slug) {
            v.push(CheckSet { key: format!("about:{slug}/3"), label: t.name.to_string(), group: "Themes: messages about each theme", question: format!("Is the message substantially about {}: {}?", t.name, lower_first(t.description)), round: 3 });
        }
    }
    v.push(CheckSet { key: "may98:3/3".into(), label: "May 1998, tier 3 (focused)".into(), group: "May 1998 flag", question: may3.into(), round: 3 });
    for slug in ["medan", "washington"] {
        if let Some(e) = lexicon::ENTITIES.iter().find(|e| e.slug == slug) {
            v.push(CheckSet { key: format!("entity:{slug}/3"), label: e.name.to_string(), group: "Names", question: format!("Does the message name {} (not an outlet, airport or namesake)?", e.name), round: 3 });
        }
    }
    for (src, label) in &titles[..2] {
        v.push(CheckSet { key: format!("title:{src}/3"), label: label.to_string(), group: "Titles", question: title_q.into(), round: 3 });
    }
    // round 1
    for t in lexicon::THEMES {
        v.push(CheckSet { key: format!("theme:{}", t.slug), label: t.name.to_string(), group: "Themes: all messages that mention each theme", question: format!("Is the message substantially about {}: {}?", t.name, lower_first(t.description)), round: 1 });
    }
    v.push(CheckSet { key: "may98:2".into(), label: "May 1998, tiers 2 and 3".into(), group: "May 1998 flag", question: may2.into(), round: 1 });
    v.push(CheckSet { key: "may98:3".into(), label: "May 1998, tier 3 (focused)".into(), group: "May 1998 flag", question: may3.into(), round: 1 });
    for slug in ["jakarta", "sukarno", "medan", "washington", "singapore", "un"] {
        if let Some(e) = lexicon::ENTITIES.iter().find(|e| e.slug == slug) {
            v.push(CheckSet { key: format!("entity:{slug}"), label: e.name.to_string(), group: "Names", question: format!("Does the message name {} (not an outlet, airport or namesake)?", e.name), round: 1 });
        }
    }
    for (src, label) in &titles {
        v.push(CheckSet { key: format!("title:{src}"), label: label.to_string(), group: "Titles", question: title_q.into(), round: 1 });
    }
    v.push(CheckSet { key: "source:text".into(), label: "Outlets read from the opening lines".into(), group: "Outlets", question: "Did the text come from the outlet shown?".into(), round: 1 });
    v
}

pub fn check_members(conn: &Connection, key: &str) -> rusqlite::Result<Vec<String>> {
    let base = check_base(key);
    let (sql, arg): (&str, String) = if let Some(slug) = base.strip_prefix("about:") {
        ("SELECT m.ak_id FROM message_themes mt JOIN messages m ON m.id = mt.msg_id JOIN themes t ON t.id = mt.theme_id WHERE t.slug = ?1 AND m.counted = 1 AND mt.about = 1", slug.into())
    } else if let Some(slug) = base.strip_prefix("theme:") {
        ("SELECT m.ak_id FROM message_themes mt JOIN messages m ON m.id = mt.msg_id JOIN themes t ON t.id = mt.theme_id WHERE t.slug = ?1 AND m.counted = 1", slug.into())
    } else if let Some(slug) = base.strip_prefix("entity:") {
        ("SELECT m.ak_id FROM message_entities me JOIN messages m ON m.id = me.msg_id JOIN entities e ON e.id = me.entity_id WHERE e.slug = ?1 AND m.counted = 1", slug.into())
    } else if let Some(src) = base.strip_prefix("title:") {
        ("SELECT ak_id FROM messages WHERE counted = 1 AND title_source = ?1", src.into())
    } else if base == "may98:2" {
        ("SELECT ak_id FROM messages WHERE counted = 1 AND may98_tier >= ?1", "2".into())
    } else if base == "may98:3" {
        ("SELECT ak_id FROM messages WHERE counted = 1 AND may98_tier = ?1", "3".into())
    } else {
        ("SELECT ak_id FROM messages WHERE counted = 1 AND source_basis = ?1", base.strip_prefix("source:").unwrap_or("").into())
    };
    conn.prepare(sql)?.query_map([arg], |r| r.get(0))?.collect()
}

/// A fixed, well-mixed sample: the members whose hash is smallest. The hash (FNV-1a with a
/// final mix) gives the same order on every machine. Samples once drawn are kept in the site
/// database (`check_samples`) and reused, so a set is always read on the same messages.
pub fn draw(key: &str, mut ids: Vec<String>, n: usize) -> Vec<String> {
    fn h(key: &str, id: &str) -> u64 {
        let mut x: u64 = 0xcbf2_9ce4_8422_2325;
        for b in key.bytes().chain(std::iter::once(0xff)).chain(id.bytes()) {
            x ^= b as u64;
            x = x.wrapping_mul(0x0000_0100_0000_01b3);
        }
        x ^= x >> 30;
        x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
        x ^ (x >> 31)
    }
    ids.sort_by_cached_key(|id| (h(key, id), id.clone()));
    ids.truncate(n);
    ids
}

/// Every check set with its sample (kept in the site store, drawn once) and the verdict
/// counts of each reviewer.
pub fn compute_checks(conn: &Connection, site: &crate::site::SiteDb) -> Result<Value> {
    let sets = check_sets();
    let mut sc = site.get()?;
    let mut out = Vec::new();
    for cs in &sets {
        let current = !sets.iter().any(|o| check_base(&o.key) == check_base(&cs.key) && o.round > cs.round);
        let members = check_members(conn, &cs.key)?;
        let total = members.len();
        let mut sample = sc.sample(&cs.key)?;
        if sample.is_empty() {
            sample = draw(&cs.key, members, 50);
            sc.store_sample(&cs.key, &sample)?;
        }
        let mut results = serde_json::Map::new();
        for (who, (y, n, u)) in sc.verdict_counts(&cs.key, &sample)? {
            results.insert(who, json!({"yes": y, "no": n, "unsure": u}));
        }
        out.push(json!({"key": cs.key, "label": cs.label, "group": cs.group, "question": cs.question, "round": cs.round, "current": current, "total": total, "sample": sample, "results": results}));
    }
    Ok(json!({"sets": out}))
}

fn day_index(d: &str) -> i64 {
    // days since 1970-01-01 (civil calendar)
    let y: i64 = d[0..4].parse().unwrap_or(1970);
    let m: i64 = d[5..7].parse().unwrap_or(1);
    let dd: i64 = d[8..10].parse().unwrap_or(1);
    let (y2, m2) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + dd - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn events(conn: &Connection) -> Result<()> {
    let mut out = Vec::new();
    for ev in lexicon::EVENTS {
        let d0 = day_index(ev.date);
        let mut st = conn.prepare(
            "SELECT m.date, m.ak_id, m.title, m.source_code FROM fts JOIN messages m ON m.id = fts.rowid
             WHERE fts MATCH ?1 AND m.counted = 1 AND m.date BETWEEN date(?2, '-120 days') AND date(?2, '+90 days') ORDER BY m.date, m.id",
        )?;
        let rows: Vec<(String, String, String, Option<String>)> = st
            .query_map(params![ev.query, ev.date], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut daily: BTreeMap<i64, i64> = BTreeMap::new();
        for (d, ..) in &rows {
            *daily.entry(day_index(d) - d0).or_default() += 1;
        }
        let before: i64 = daily.range(-120..-30).map(|(_, n)| *n).sum();
        let baseline = before as f64 / 90.0;
        let after30: i64 = daily.range(0..30).map(|(_, n)| *n).sum();
        let peak = daily.range(0..=90).max_by_key(|(_, n)| **n).map(|(k, n)| json!({"day": k, "n": n}));
        // the first report: a title that matches, within two weeks; otherwise the first message that mentions it
        let by_title: Option<(String, String, String, Option<String>)> = conn
            .query_row(
                "SELECT m.date, m.ak_id, m.title, m.source_code FROM fts JOIN messages m ON m.id = fts.rowid
                 WHERE fts MATCH ?1 AND m.counted = 1 AND m.date BETWEEN ?2 AND date(?2, '+14 days') ORDER BY m.date, m.id LIMIT 1",
                params![format!("title : ({})", ev.query), ev.date],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .unwrap_or(None);
        let first = match by_title {
            Some(r) => Some(json!({"date": r.0, "ak_id": r.1, "title": r.2, "source": r.3, "lag_days": day_index(&r.0) - d0, "basis": "title"})),
            None => rows.iter().find(|r| day_index(&r.0) >= d0).map(|r| json!({"date": r.0, "ak_id": r.1, "title": r.2, "source": r.3, "lag_days": day_index(&r.0) - d0, "basis": "text"})),
        };
        let early: Vec<Value> = rows.iter().filter(|r| day_index(&r.0) >= d0).take(5).map(|r| json!({"date": r.0, "ak_id": r.1, "title": r.2, "source": r.3})).collect();
        // capture density around the date: all messages, to tell silence from missing captures
        let all30: i64 = conn.query_row("SELECT count(*) FROM messages WHERE counted = 1 AND date BETWEEN ?1 AND date(?1, '+29 days')", [ev.date], |r| r.get(0))?;
        out.push(json!({
            "slug": ev.slug, "date": ev.date, "name": ev.name, "description": ev.description, "query": ev.query,
            "daily": daily.range(-30..=90).map(|(k, n)| json!([k, n])).collect::<Vec<_>>(),
            "baseline_per_day": baseline, "after30": after30, "all_after30": all30,
            "lift": if baseline > 0.0 { after30 as f64 / 30.0 / baseline } else { 0.0 },
            "peak": peak, "first": first, "early": early,
        }));
    }
    put(conn, "events", &json!({"events": out}))
}

/// The May 1998 case study, by tier: 1 mentions sexual violence, 2 in the same paragraph as
/// the riots or the inquiries, 3 focused on it (see `flags`).
fn may1998(conn: &Connection) -> Result<()> {
    let mut monthly = Vec::new();
    let mut st = conn.prepare(
        "SELECT ym, sum(sv_flag), sum(may98_tier = 1), sum(may98_tier = 2), sum(may98_tier = 3), count(*) FROM messages
         WHERE counted = 1 AND ym BETWEEN '1998-01' AND '2002-02' GROUP BY ym",
    )?;
    for r in st.query_map([], |r| Ok(json!({"ym": r.get::<_, String>(0)?, "sv": r.get::<_, i64>(1)?, "t1": r.get::<_, i64>(2)?, "t2": r.get::<_, i64>(3)?, "t3": r.get::<_, i64>(4)?, "all": r.get::<_, i64>(5)?})))? {
        monthly.push(r?);
    }
    let tiers = json!({
        "mention": scalar(conn, "SELECT count(*) FROM messages WHERE counted = 1 AND may98_tier >= 1"),
        "paragraph": scalar(conn, "SELECT count(*) FROM messages WHERE counted = 1 AND may98_tier >= 2"),
        "focused": scalar(conn, "SELECT count(*) FROM messages WHERE counted = 1 AND may98_tier = 3"),
        "sv_all_years": scalar(conn, "SELECT count(*) FROM messages WHERE counted = 1 AND sv_flag = 1"),
    });
    let mut srcs = Vec::new();
    let mut st = conn.prepare("SELECT coalesce(source_code,'(no code)'), max(source_name), count(*), sum(may98_tier = 3) FROM messages WHERE counted = 1 AND may98_tier >= 2 GROUP BY 1 ORDER BY 3 DESC LIMIT 15")?;
    for r in st.query_map([], |r| Ok(json!({"code": r.get::<_, String>(0)?, "name": r.get::<_, Option<String>>(1)?, "n": r.get::<_, i64>(2)?, "focused": r.get::<_, i64>(3)?})))? {
        srcs.push(r?);
    }
    let mut first = Vec::new();
    let mut st = conn.prepare("SELECT ak_id, date, title, source_code, lang, may98_tier FROM messages WHERE counted = 1 AND may98_tier >= 2 ORDER BY date, id LIMIT 30")?;
    for r in st.query_map([], |r| Ok(json!({"ak_id": r.get::<_, String>(0)?, "date": r.get::<_, Option<String>>(1)?, "title": r.get::<_, String>(2)?, "source": r.get::<_, Option<String>>(3)?, "lang": r.get::<_, String>(4)?, "tier": r.get::<_, i64>(5)?})))? {
        first.push(r?);
    }
    let mut focused = Vec::new();
    let mut st = conn.prepare("SELECT ak_id, date, title, source_code FROM messages WHERE counted = 1 AND may98_tier = 3 ORDER BY date, id")?;
    for r in st.query_map([], |r| Ok(json!({"ak_id": r.get::<_, String>(0)?, "date": r.get::<_, Option<String>>(1)?, "title": r.get::<_, String>(2)?, "source": r.get::<_, Option<String>>(3)?})))? {
        focused.push(r?);
    }
    let lang: Vec<(String, i64)> = conn.prepare("SELECT lang, count(*) FROM messages WHERE counted = 1 AND may98_tier >= 2 GROUP BY 1 ORDER BY 2 DESC")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let terms = ["perkosaan", "diperkosa", "pemerkosaan", "rape", "raped", "tim relawan", "tgpf", "martadinata", "komnas perempuan", "rekayasa", "fabricat", "hoax", "fitnah", "provokator", "tionghoa", "kalyanamitra", "sandyawan", "habibie"];
    let mut term_counts = Vec::new();
    for t in terms {
        let n: i64 = conn.query_row("SELECT count(*) FROM messages WHERE counted = 1 AND may98_tier >= 2 AND instr(lower(body), ?1) > 0", [t], |r| r.get(0))?;
        term_counts.push(json!({"term": t, "n": n}));
    }
    put(conn, "may1998", &json!({"tiers": tiers, "monthly": monthly, "sources": srcs, "first": first, "focused": focused, "languages": lang, "terms": term_counts}))
}

fn left1965(conn: &Connection) -> Result<()> {
    let terms = [("gerwani", "Gerwani"), ("pki", "PKI"), ("g30s", "G30S"), ("1965", "1965"), ("pramoedya", "Pramoedya"), ("tapol", "tapol"), ("pulau buru", "Pulau Buru"), ("komunis", "komunis"), ("communist", "communist")];
    let mut series = Vec::new();
    for (t, label) in terms {
        let mut yearly = BTreeMap::new();
        let mut st = conn.prepare("SELECT m.year, count(*) FROM fts JOIN messages m ON m.id = fts.rowid WHERE fts MATCH ?1 AND m.counted = 1 GROUP BY m.year")?;
        let q = format!("\"{}\"", t);
        for r in st.query_map([q], |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, i64>(1)?)))? {
            let (y, n) = r?;
            if let Some(y) = y { yearly.insert(y.to_string(), n); }
        }
        series.push(json!({"term": t, "label": label, "yearly": yearly}));
    }
    let mut ex = Vec::new();
    let mut st = conn.prepare("SELECT m.ak_id, m.date, m.title, m.source_code FROM fts JOIN messages m ON m.id = fts.rowid WHERE fts MATCH 'gerwani' AND m.counted = 1 ORDER BY m.date LIMIT 30")?;
    for r in st.query_map([], |r| Ok(json!({"ak_id": r.get::<_, String>(0)?, "date": r.get::<_, Option<String>>(1)?, "title": r.get::<_, String>(2)?, "source": r.get::<_, Option<String>>(3)?})))? {
        ex.push(r?);
    }
    put(conn, "left1965", &json!({"series": series, "gerwani_messages": ex}))
}

/// List machinery, mail software and MacDougall's posting templates: frequent, but they
/// describe the medium, not the news.
static FORMAT: once_cell::sync::Lazy<AHashSet<&'static str>> = once_cell::sync::Lazy::new(|| {
    "brief byline type dateline datelined original story remark language english none text source date full abridged written topic title \
     usenet sprintlink path newsgroups newsgroup xref culture delphi digex greennet reston howland unsub admin subscribe unsubscribe \
     berlangganan ongkos clark online cost alamat radix saltmine apakabar message messages forwarded forward macdougall john maillist \
     iname indopubs publications lists list http https html return database databases mailing available center homepage page links \
     home back closes reminder email e-mail mail sender subject reply organization organisation posted post posting article fwd \
     editor moderator conference igc peacenet access igc.apc.org owner archive archives web site website www kirim send please \
     silakan terima kasih thanks salam regards sincerely received hamline ohio server listserv majordomo bulk mailer \
     one-time lifetime check donation subscription subscriptions arrangements parchives varchives darchives tarchives esosoft basisdata \
     search-all active uunet idndprin dprin link ismap embed address gratis hotmail siarlist minihub attachments attachment carries comr \
     translations facsimile precedence copyright accessible phased definitely evolve".split_whitespace().collect()
});

/// Words most characteristic of each year compared with all other years (Dunning's
/// log-likelihood, G2), from canonical messages. Stop words, numbers and words
/// seen fewer than 40 times are left out.
fn distinctive_terms(conn: &Connection) -> Result<()> {
    let rows: Vec<(i64, String)> = {
        let mut st = conn.prepare("SELECT year, title || ' ' || body FROM messages WHERE counted = 1 AND year IS NOT NULL")?;
        let it = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        it.collect::<rusqlite::Result<_>>()?
    };
    let years: Vec<i64> = (1990..=2002).collect();
    let counts: Vec<AHashMap<String, u32>> = years
        .par_iter()
        .map(|y| {
            let mut m: AHashMap<String, u32> = AHashMap::new();
            for (yy, t) in rows.iter() {
                if yy != y { continue; }
                // count a word once per message, so one long document cannot dominate a year
                let mut seen: AHashSet<String> = AHashSet::new();
                for w in text::tokens(t) {
                    if w.len() < 4 || text::is_stop(&w) || w.chars().any(|c| c.is_ascii_digit()) || w.contains('\'') || FORMAT.contains(w.as_str()) || w.ends_with("-l") || w.starts_with("indonesia-") || w.contains("posting") { continue; }
                    if seen.insert(w.clone()) {
                        *m.entry(w).or_default() += 1;
                    }
                }
            }
            m
        })
        .collect();
    let docs_per_year: Vec<f64> = years.iter().map(|y| rows.iter().filter(|r| r.0 == *y).count() as f64).collect();
    let total_docs: f64 = docs_per_year.iter().sum();
    let mut all: AHashMap<&str, u32> = AHashMap::new();
    for m in &counts {
        for (w, n) in m {
            *all.entry(w.as_str()).or_default() += n;
        }
    }
    let mut out = Vec::new();
    for (i, y) in years.iter().enumerate() {
        let a_docs = docs_per_year[i];
        if a_docs < 50.0 { continue; }
        let b_docs = total_docs - a_docs;
        let mut scored: Vec<(f64, &str, u32)> = counts[i]
            .iter()
            .filter_map(|(w, &a)| {
                let tot = *all.get(w.as_str()).unwrap_or(&0);
                if tot < 40 || a < 8 { return None; }
                let b = (tot - a) as f64;
                let a = a as f64;
                // expected counts
                let e1 = a_docs * (a + b) / total_docs;
                let e2 = b_docs * (a + b) / total_docs;
                if a <= e1 { return None; }
                let g2 = 2.0 * (a * (a / e1).ln() + if b > 0.0 { b * (b / e2).ln() } else { 0.0 });
                Some((g2, w.as_str(), a as u32))
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        out.push(json!({"year": y, "docs": a_docs, "terms": scored.iter().take(30).map(|(g, w, n)| json!({"w": w, "g2": (g * 10.0).round() / 10.0, "docs": n})).collect::<Vec<_>>()}));
    }
    put(conn, "distinctive_terms", &json!({"years": out}))
}
