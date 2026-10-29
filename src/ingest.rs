//! Builds the database from the harvested text files: parse, clean, title, classify,
//! de-duplicate, thread, index. Runs in parallel; the whole corpus takes a few minutes.
//!
//! The database is rebuilt from scratch into a temporary file, which then replaces the old
//! one. Accounts, bookmarks, notes and editors' flags live in a separate file (site.db) that
//! a rebuild never touches.

use crate::{clean, db, flags, listings, parse, sources, text, titles};
use ahash::{AHashMap, AHashSet, RandomState};
use rayon::prelude::*;
use rusqlite::params;
use std::hash::{BuildHasher, Hash, Hasher};
use std::path::Path;
use std::time::Instant;

struct Proc {
    raw: parse::RawMsg,
    kind: parse::IdKind,
    collection: &'static str,
    date: Option<String>,
    date_source: &'static str,
    date_note: Option<String>,
    sub: sources::Subject,
    topic: Option<String>,
    title: String,
    title_source: &'static str,
    code: Option<String>,
    source_basis: &'static str,
    source_name: Option<String>,
    source_cat: String,
    lang: String,
    mtype: &'static str,
    cl: clean::Cleaned,
    stems: String,
    raw_z: Vec<u8>,
    words: usize,
    exact: u64,
    simhash: u64,
    fl: flags::Flags,
}

fn hasher() -> RandomState {
    RandomState::with_seeds(0x5eed_1990, 0xa9a_ca8a, 0x0c70_1990, 0x2002_0202)
}

fn fingerprints(body: &str) -> (u64, u64, usize) {
    let words: Vec<String> = text::tokens(body).filter(|w| w.len() > 1).collect();
    let rs = hasher();
    let mut h = rs.build_hasher();
    words.hash(&mut h);
    let exact = h.finish();
    let mut v = [0i32; 64];
    for w in words.windows(3) {
        let mut h = rs.build_hasher();
        w.hash(&mut h);
        let x = h.finish();
        for (b, slot) in v.iter_mut().enumerate() {
            if x >> b & 1 == 1 { *slot += 1 } else { *slot -= 1 }
        }
    }
    let mut sim = 0u64;
    for (b, slot) in v.iter().enumerate() {
        if *slot > 0 {
            sim |= 1 << b;
        }
    }
    (exact, sim, words.len())
}

fn header_date(headers: &str) -> Option<String> {
    headers
        .lines()
        .filter_map(|l| {
            let l = l.trim_start_matches(|c| c == '>' || c == ' ');
            l.strip_prefix("Date:").or_else(|| l.strip_prefix("date:")).and_then(|v| parse::date_from_line(v))
        })
        .next()
}

/// A conference topic with this many replies or more is a recurring heading, not a title.
const GENERIC_TOPIC: usize = 10;

fn process(mut r: parse::RawMsg, generic: &AHashSet<String>) -> Proc {
    let kind = parse::id_kind(&r.ak_id);
    let sub = sources::parse_subject(&r.subject);
    let cl = clean::clean(&r.body);
    let line_date = parse::date_from_line(&r.date_line);

    // dates: a day folder fixes the day; elsewhere the date lines decide
    let (mut date, mut date_source, mut date_note) = (None, "none", None);
    match kind {
        parse::IdKind::Day => {
            date = parse::id_date(&r.ak_id);
            date_source = "id";
            if let (Some(d), Some(l)) = (&date, &line_date) {
                if day_gap(Some(d), Some(l)) > 1 {
                    date_note = Some(format!("The message's own date line reads {l}"));
                }
            }
        }
        _ => {
            if let Some(d) = line_date.clone() {
                date = Some(d);
                date_source = "date_line";
            } else if let Some(d) = parse::date_from_line(&cl.orig_date) {
                date = Some(d);
                date_source = "inner_date";
            } else if let Some(d) = header_date(&cl.headers) {
                date = Some(d);
                date_source = "header";
            } else if kind == parse::IdKind::Month {
                date_note = Some("Only the month is known".into());
            }
        }
    }

    let collection = if r.ak_id.starts_with("AKL-iri") || sub.list_tag.as_deref() == Some("iri-l") {
        "iri-l"
    } else if kind == parse::IdKind::List {
        "list"
    } else {
        "basisdata"
    };

    // titles: the subject, or a better one from inside the message
    let inner = if cl.orig_subject.trim().is_empty() { None } else { Some(sources::parse_subject(&cl.orig_subject)) };
    // a reply under a recurring topic, or one that carries its own subject inside, takes its title
    // from the text; a reply under a rare topic keeps the topic as its title
    let has_inner = inner.as_ref().map(|s| !s.title.trim().is_empty()).unwrap_or(false);
    let as_topic = sub.topic_reply && (has_inner || generic.contains(&sources::topic_name(&r.subject)));
    let derived = titles::derive(&sub.title, as_topic, inner.as_ref().map(|s| s.title.as_str()), &cl.lead, &cl.text);
    let topic = if sub.topic_reply { Some(sources::topic_name(&r.subject)).filter(|t| !t.is_empty()) } else { None };

    let mut code = sub.code.clone();
    let mut source_basis = if code.is_some() { "subject" } else { "" };
    if let Some(c) = inner.as_ref().and_then(|s| s.code.clone()) {
        if code.is_none() || sub.topic_reply {
            code = Some(c);
            source_basis = "inner_subject";
        }
    }
    if code.is_none() {
        if let Some(c) = sources::code_from_url(&cl.x_url) {
            code = Some(c.to_string());
            source_basis = "url";
        }
    }
    if code.is_none() {
        if let Some(c) = sources::code_from_body(&cl.text) {
            code = Some(c.to_string());
            source_basis = "text";
        }
    }
    let (source_name, source_cat) = match code.as_deref().and_then(|c| sources::BY_CODE.get(c)) {
        Some(s) => (Some(s.name.to_string()), s.category.to_string()),
        None => (None, if code.is_some() { "other_code".to_string() } else { "uncoded".to_string() }),
    };
    let lang = text::detect_lang(&format!("{} {}", derived.title, cl.text));
    let mtype = sources::msg_type(&sub, &source_cat, &derived.title.to_lowercase());
    let stems = text::stems(&format!("{} {}", derived.title, cl.text));
    let raw_z = zstd::encode_all(r.body.as_bytes(), 6).unwrap_or_default();
    let (exact, simhash, words) = fingerprints(&cl.text);
    let fl = flags::flags(&derived.title, &cl.text, date.as_deref());
    r.body = String::new(); // kept compressed in raw_z
    Proc {
        raw: r, kind, collection, date, date_source, date_note, sub, topic, title: derived.title, title_source: derived.source,
        code, source_basis, source_name, source_cat, lang, mtype, cl, stems, raw_z, words, exact, simhash, fl,
    }
}

struct Uf(Vec<usize>);
impl Uf {
    fn find(&mut self, x: usize) -> usize {
        let mut r = x;
        while self.0[r] != r { r = self.0[r]; }
        let mut y = x;
        while self.0[y] != r { let n = self.0[y]; self.0[y] = r; y = n; }
        r
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            // the earlier index (basisdata first, then date) stays the root
            if ra < rb { self.0[rb] = ra } else { self.0[ra] = rb }
        }
    }
}

pub fn run(input: &Path, db_path: &Path) -> anyhow_lite::Result<()> {
    let t0 = Instant::now();
    // an archive database from an older version may still hold accounts: move them to site.db first
    if db_path.exists() && crate::site::env_url().is_none() {
        let site = crate::site::site_path(db_path, None);
        if let Err(e) = crate::site::SiteDb::open(None, &site, Some(db_path)) {
            eprintln!("note: could not prepare {}: {e}", site.display());
        }
    }
    let files = parse::find_files(input);
    eprintln!("{} files", files.len());
    let mut raws: Vec<parse::RawMsg> = files.par_iter().flat_map(|f| parse::parse_file(f)).collect();
    // one record per id (a retried capture may also exist in a month file)
    let mut seen = AHashMap::new();
    raws.retain(|r| seen.insert(r.ak_id.clone(), ()).is_none());
    eprintln!("{} messages parsed in {:.1?}", raws.len(), t0.elapsed());
    // Conference topics that recur ("Issues: Law & Liberty", "RUBRIK - Lelucon") say little about
    // a single reply, so its title is looked for in the text; a topic with few replies is itself
    // the best title ("FEER - Wiranto's Way (r)").
    let mut tcount: AHashMap<String, usize> = AHashMap::new();
    for r in &raws {
        if sources::parse_subject(&r.subject).topic_reply {
            *tcount.entry(sources::topic_name(&r.subject)).or_default() += 1;
        }
    }
    let generic: AHashSet<String> = tcount.into_iter().filter(|(_, n)| *n >= GENERIC_TOPIC).map(|(k, _)| k).collect();
    eprintln!("{} recurring conference topics", generic.len());
    let mut procs: Vec<Proc> = raws.into_par_iter().map(|r| process(r, &generic)).collect();
    fill_list_dates(&mut procs);
    procs.sort_by(|a, b| {
        let ka = (a.collection != "basisdata", a.date.clone().unwrap_or_default(), a.raw.ak_id.clone());
        let kb = (b.collection != "basisdata", b.date.clone().unwrap_or_default(), b.raw.ak_id.clone());
        ka.cmp(&kb)
    });
    eprintln!("cleaned and classified in {:.1?}", t0.elapsed());

    // duplicates: exact text, then near-identical (simhash, 4 bands of 16 bits)
    let n = procs.len();
    let mut uf = Uf((0..n).collect());
    let mut by_exact: AHashMap<u64, usize> = AHashMap::new();
    for (i, p) in procs.iter().enumerate() {
        if p.words < 30 { continue; }
        match by_exact.get(&p.exact) {
            Some(&j) => uf.union(j, i),
            None => { by_exact.insert(p.exact, i); }
        }
    }
    for band in 0..4 {
        let mut buckets: AHashMap<u16, Vec<usize>> = AHashMap::new();
        for (i, p) in procs.iter().enumerate() {
            if p.words < 60 { continue; }
            buckets.entry((p.simhash >> (band * 16)) as u16).or_default().push(i);
        }
        for (_, v) in buckets {
            if v.len() < 2 || v.len() > 400 { continue; }
            for a in 0..v.len() {
                for b in a + 1..v.len() {
                    let (pa, pb) = (&procs[v[a]], &procs[v[b]]);
                    let ratio = pa.words.min(pb.words) as f64 / pa.words.max(pb.words) as f64;
                    if ratio > 0.85 && (pa.simhash ^ pb.simhash).count_ones() <= 3 {
                        uf.union(v[a], v[b]);
                    }
                }
            }
        }
    }
    let dup_root: Vec<usize> = (0..n).map(|i| uf.find(i)).collect();
    let n_dups = (0..n).filter(|&i| dup_root[i] != i).count();
    eprintln!("{} duplicates found", n_dups);
    let n_from_dups = titles_from_copies(&mut procs, &dup_root);
    eprintln!("{} titles completed from other copies", n_from_dups);

    // threads: same subject, gaps of at most 45 days, at least one "Re:"; topic replies are
    // grouped by their topic instead. A thread is named by the ID of its first message.
    let mut groups: AHashMap<String, Vec<usize>> = AHashMap::new();
    for (i, p) in procs.iter().enumerate() {
        if p.sub.topic_reply && p.title_source != "inner_subject" { continue; }
        let k = thread_key(&p.title);
        if k.len() < 12 || p.source_cat == "list_rubric" || p.collection == "iri-l" { continue; }
        groups.entry(k).or_default().push(i);
    }
    let mut thread_of: Vec<Option<usize>> = vec![None; n];
    let mut n_threads = 0;
    for (_, mut v) in groups {
        if v.len() < 2 { continue; }
        v.sort_by(|&a, &b| procs[a].date.cmp(&procs[b].date).then(a.cmp(&b)));
        let mut start = 0;
        for k in 1..=v.len() {
            let gap_ok = k < v.len() && day_gap(procs[v[k - 1]].date.as_deref(), procs[v[k]].date.as_deref()) <= 45;
            if !gap_ok {
                let seg = &v[start..k];
                let has_reply = seg.iter().any(|&i| procs[i].sub.is_reply);
                if seg.len() >= 2 && has_reply {
                    let root = seg[0];
                    for &i in seg { thread_of[i] = Some(root); }
                    n_threads += 1;
                }
                start = k;
            }
        }
    }
    eprintln!("{} threads", n_threads);

    // the database's own folder listings: which messages it held, and which no capture keeps
    let lst = listings::load(input);
    let present: AHashSet<&str> = procs.iter().filter(|p| p.collection == "basisdata").map(|p| p.raw.ak_id.as_str()).collect();
    let mut list_copies: AHashMap<&str, Vec<(&str, String)>> = AHashMap::new();
    for p in procs.iter().filter(|p| p.collection == "list") {
        if let Some(d) = p.date.as_deref() {
            list_copies.entry(d).or_default().push((p.raw.ak_id.as_str(), listings::title_key(&p.raw.subject)));
        }
    }
    let mut lost: Vec<(String, Option<String>, u32, String, String, String, String, Option<String>)> = Vec::new();
    for (ak, when, num, size, desc, src, ts) in listings::listed_ids(&lst) {
        if present.contains(ak.as_str()) {
            continue;
        }
        let day = if when.len() == 10 { Some(when.clone()) } else { None };
        // a possible copy in the list archives: same days, subject starting with the same words
        let key = listings::title_key(desc);
        let mut maybe: Vec<&str> = Vec::new();
        if let (Some(d), true) = (&day, key.len() >= 8) {
            let z = day_number(d);
            for off in -1..=2 {
                if let Some(v) = list_copies.get(date_of_day_number(z + off).as_str()) {
                    maybe.extend(v.iter().filter(|(_, k)| k.starts_with(&key)).map(|(a, _)| *a));
                }
            }
        }
        maybe.truncate(5);
        lost.push((ak, day, num, size.to_string(), desc.to_string(), src.to_string(), ts.to_string(), if maybe.is_empty() { None } else { Some(maybe.join(",")) }));
    }
    eprintln!("listings: {} day folders and {} months; {} listed messages were never captured", lst.days.len(), lst.months.len(), lost.len());

    // write into a new file, then swap it in, so a failed run leaves the old database working
    let tmp = db_path.with_extension("db.building");
    for p in [tmp.clone(), tmp.with_extension("building-wal"), tmp.with_extension("building-shm")] {
        let _ = std::fs::remove_file(p);
    }
    let mut conn = db::open_rw(&tmp)?;
    conn.execute_batch("PRAGMA synchronous = OFF; PRAGMA journal_mode = MEMORY;")?;
    let tx = conn.transaction()?;
    {
        let mut ins = tx.prepare(
            "INSERT INTO messages (id, ak_id, collection, list_tag, date, ym, year, date_source, date_note, subject, title, title_source, topic, is_topic_reply,
              sender, orig_from, orig_date, orig_subject, source_code, source_basis, source_name, source_cat, lang, msg_type, is_reply, dup_of, counted,
              thread_id, chars, words, headers, body, raw_z, orig_url, wayback_url, repaired, html_fixed, sv_flag, may98_tier, may98_flag, file)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33,?34,?35,?36,?37,?38,?39,?40,?41)",
        )?;
        let mut ins_fts = tx.prepare("INSERT INTO fts(rowid, title, body, stems) VALUES (?1, ?2, ?3, ?4)")?;
        for (i, p) in procs.iter().enumerate() {
            let id = (i + 1) as i64;
            let ym = p.date.as_ref().map(|d| d[..7].to_string()).or_else(|| parse::id_month(&p.raw.ak_id));
            let year = ym.as_ref().and_then(|d| d[..4].parse::<i64>().ok());
            let dup = if dup_root[i] != i { Some(dup_root[i] as i64 + 1) } else { None };
            let counted = dup.is_none() && p.collection != "iri-l";
            let thread = thread_of[i].map(|r| procs[r].raw.ak_id.clone());
            let list_tag = p.sub.list_tag.clone().or_else(|| if p.kind == parse::IdKind::List { parse::list_archive(&p.raw.ak_id) } else { None });
            ins.execute(params![
                id, p.raw.ak_id, p.collection, list_tag, p.date, ym, year, p.date_source, p.date_note, p.raw.subject, p.title, p.title_source, p.topic,
                p.sub.topic_reply as i64, p.raw.from, p.cl.orig_from, p.cl.orig_date, p.cl.orig_subject, p.code, p.source_basis, p.source_name, p.source_cat,
                p.lang, p.mtype, p.sub.is_reply as i64, dup, counted as i64, thread, p.cl.text.chars().count() as i64, p.words as i64,
                p.cl.headers, p.cl.text, p.raw_z, p.raw.orig_url, p.raw.wayback_url, p.cl.repaired_chars as i64, p.cl.html_fixed as i64,
                p.fl.sv as i64, p.fl.may_tier as i64, (p.fl.may_tier >= 2) as i64, p.raw.file
            ])?;
            let fts_title = match &p.topic { Some(t) => format!("{} {}", p.title, t), None => p.title.clone() };
            ins_fts.execute(params![id, fts_title, p.cl.text, p.stems])?;
        }
    }
    {
        let mut idl = tx.prepare("INSERT INTO day_listings(day, listed, src, ts) VALUES (?1, ?2, ?3, ?4)")?;
        for (day, d) in &lst.days {
            idl.execute(params![day, d.files.len() as i64, d.src, d.ts])?;
        }
        let mut imf = tx.prepare("INSERT INTO month_folders(ym, days, src, ts) VALUES (?1, ?2, ?3, ?4)")?;
        for (ym, m) in &lst.months {
            imf.execute(params![ym, m.dirs.join(","), m.src, m.ts])?;
        }
        let mut il = tx.prepare("INSERT OR IGNORE INTO lost(ak_id, date, num, size, title, src, ts, maybe) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")?;
        for (ak, day, num, size, desc, src, ts, maybe) in &lost {
            il.execute(params![ak, day, *num as i64, size, desc, src, ts, maybe])?;
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES ('built_at', datetime('now')), ('n_messages', ?1), ('n_duplicates', ?2), ('n_threads', ?3), ('stemmer', 'sastrawi-rs 0.5.3 + overrides'), ('schema', ?4)",
        params![n as i64, n_dups as i64, n_threads as i64, db::SCHEMA_VERSION],
    )?;
    tx.commit()?;
    eprintln!("database written in {:.1?}", t0.elapsed());
    conn.execute_batch("INSERT INTO fts(fts) VALUES('optimize'); ANALYZE; PRAGMA journal_mode = DELETE;")?;
    drop(conn);
    for ext in ["db-wal", "db-shm"] {
        let _ = std::fs::remove_file(db_path.with_extension(ext));
    }
    std::fs::rename(&tmp, db_path)?;
    eprintln!("indexes optimised and database in place in {:.1?}", t0.elapsed());
    Ok(())
}

/// Days since 1970-01-01 for "YYYY-MM-DD" (civil calendar), and back.
fn day_number(d: &str) -> i64 {
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

fn date_of_day_number(z: i64) -> String {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

fn thread_key(title: &str) -> String {
    let k = titles::without_part(&title.to_lowercase());
    k.split_whitespace().collect::<Vec<_>>().join(" ").trim_matches(|c: char| !c.is_alphanumeric()).to_string()
}

/// Copies of one text keep different subjects: a reply filed under a topic in the database
/// is often a full subject in the list archives, and a cut subject is often whole elsewhere.
fn titles_from_copies(procs: &mut [Proc], dup_root: &[usize]) -> usize {
    let mut groups: AHashMap<usize, Vec<usize>> = AHashMap::new();
    for (i, r) in dup_root.iter().enumerate() {
        groups.entry(*r).or_default().push(i);
    }
    let mut changes: Vec<(usize, String)> = Vec::new();
    for (_, members) in groups.into_iter().filter(|(_, v)| v.len() > 1) {
        // the best title among the copies: a real subject, preferably the longest that extends the others
        let real: Vec<usize> = members.iter().copied().filter(|&i| matches!(procs[i].title_source, "subject" | "inner_subject" | "body_prefix") && !procs[i].sub.topic_reply).collect();
        for &i in &members {
            let p = &procs[i];
            let better = real.iter().filter(|&&j| j != i).map(|&j| &procs[j].title).filter(|t| {
                match p.title_source {
                    "subject" | "inner_subject" | "body_prefix" => titles::extends(&p.title, t),
                    // topic replies: any real subject of a copy is better than a headline guess
                    _ => !t.is_empty(),
                }
            }).max_by_key(|t| t.chars().count());
            if let Some(t) = better {
                changes.push((i, t.clone()));
            }
        }
    }
    let n = changes.len();
    for (i, t) in changes {
        procs[i].title = t;
        procs[i].title_source = "duplicate";
    }
    n
}

/// List-archive messages without any date take the date of the neighbouring messages in the
/// same archive when those agree within a week.
fn fill_list_dates(procs: &mut [Proc]) {
    let mut by_archive: AHashMap<String, Vec<(u32, usize)>> = AHashMap::new();
    for (i, p) in procs.iter().enumerate() {
        if p.kind == parse::IdKind::List {
            if let (Some(a), Some(n)) = (parse::list_archive(&p.raw.ak_id), parse::id_number(&p.raw.ak_id)) {
                by_archive.entry(a).or_default().push((n, i));
            }
        }
    }
    for (_, mut v) in by_archive {
        v.sort();
        let dates: Vec<Option<String>> = v.iter().map(|&(_, i)| procs[i].date.clone()).collect();
        for k in 0..v.len() {
            if dates[k].is_some() { continue; }
            let prev = (0..k).rev().find_map(|j| dates[j].clone());
            let next = (k + 1..v.len()).find_map(|j| dates[j].clone());
            let i = v[k].1;
            match (prev, next) {
                (Some(a), Some(b)) if day_gap(Some(&a), Some(&b)) <= 7 => {
                    procs[i].date_note = Some(if a == b { format!("Dated from the neighbouring messages ({a})") } else { format!("Dated from the neighbouring messages ({a} to {b})") });
                    procs[i].date = Some(a);
                    procs[i].date_source = "neighbours";
                }
                (Some(a), Some(b)) => procs[i].date_note = Some(format!("Undated; the neighbouring messages date from {a} and {b}")),
                _ => procs[i].date_note = Some("Undated".into()),
            }
        }
    }
}

fn day_num(d: &str) -> i64 {
    let y: i64 = d.get(0..4).and_then(|s| s.parse().ok()).unwrap_or(0);
    let m: i64 = d.get(5..7).and_then(|s| s.parse().ok()).unwrap_or(1);
    let dd: i64 = d.get(8..10).and_then(|s| s.parse().ok()).unwrap_or(1);
    y * 372 + m * 31 + dd
}

fn day_gap(a: Option<&str>, b: Option<&str>) -> i64 {
    match (a, b) {
        (Some(a), Some(b)) => (day_num(b) - day_num(a)).abs(),
        _ => 9999,
    }
}

/// Minimal error type so the binary needs no extra error crates.
pub mod anyhow_lite {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
}
