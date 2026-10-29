//! The catalogue in the Gerwani Archive index format: one row per message with the same
//! columns as the Gerwani Archive Index and the May 1998 archive, plus the fields this archive
//! adds (outlet, themes, conference topic, title source, May 1998 tier).
//!
//! Writes two workbooks (the Items sheet is split by period to keep each file small) and a
//! gzip-compressed CSV with every row:
//!
//!   Apakabar Archive Index.xlsx                         README, Items 1990-1997, summaries
//!   Apakabar Archive Index - Items 1998-2002 and lists.xlsx
//!   Apakabar Archive Index - Items.csv.gz

use crate::{analyze, db, ingest::anyhow_lite::Result, sources};
use ahash::AHashMap;
use rust_xlsxwriter::{Color, Format, Workbook, Worksheet};
use serde_json::Value;
use std::io::Write;
use std::path::Path;

const COLS: &[&str] = &[
    "item_id", "group_id", "related_items", "slug", "organisation", "org_code", "also_relevant_to", "title", "title_en", "summary_en", "summary_id", "date", "year", "period",
    "creator", "publication", "document_type", "document_type_detail", "language", "collection", "section", "subcollection", "file_name", "file_path", "file_path_in_parent",
    "file_type", "pages", "size_mb", "people", "organisations", "places", "topics", "relevance", "relevance_note", "identity_check", "source_archive", "source_url",
    "access_rights", "citation_apa", "in_text", "ocr_quality", "full_text_available", "text_status", "text_file", "full_text_workbook", "clean_text_file", "text_proofread",
    "translation_available", "translation_file", "translation_note", "key_passage", "summary_basis", "annotation_level", "perspective", "source_role", "featured",
    "country_of_publication", "decade", "date_precision", "event_ids", "publication_id", "person_ids", "organisation_ids", "place_ids", "topic_ids", "tag_ids", "tags",
    "keywords_en", "keywords_id", "source_id", "collection_id", "file_visibility", "drive_file_id", "drive_url", "drive_folder_url", "clean_text_drive_url",
    "translation_drive_url", "auto_filled",
];
const EXTRA: &[&str] = &["list_name", "original_url", "wayback_url", "capture_timestamp", "chars", "in_archive", "status", "sexual_violence_flag", "may1998_flag"];
const ADDED: &[&str] = &[
    "subject_as_posted", "title_source", "conference_topic", "outlet_code", "outlet", "outlet_category", "outlet_basis", "message_type", "duplicate_of", "thread_id",
    "may1998_tier", "date_note", "words",
];

fn dictionary(c: &str) -> &'static str {
    match c {
        "item_id" => "Permanent ID. AK-YYYY-MM-DD-NNNN: database day folder and message number; AK-YYYY-MM-NNNN: the month folder of 4 January to 11 March 2000; AKL-<archive>-NNNN: indopubs.com list archives",
        "group_id" => "The monthly file or list archive the message belongs to",
        "related_items" => "Other copies of the same text (exact or near-exact duplicates)",
        "title" => "Best title: the subject line, or a title from the text where the subject was cut short or only named a conference topic (see title_source)",
        "creator" => "From line of the database page (usually the list address)",
        "people" => "People named, from the archive's list of names and spellings",
        "organisations" => "Organisations named, from the archive's list",
        "places" => "Places named, from the archive's list",
        "topics" => "Themes the message is about: it mentions the theme's terms and the model judges the theme a real subject of the text (see the Themes sheet and the precision checks)",
        "date" => "Day of posting: the day folder for database messages, the date line otherwise",
        "date_precision" => "day, month (month folder without a date line) or undetermined",
        "sexual_violence_flag" => "yes when the message mentions rape or sexual violence",
        "may1998_flag" => "yes for May 1998 tiers 2 and 3 (from 13 May 1998, sexual violence in the same paragraph as the riots or the inquiries)",
        "subject_as_posted" => "Subject line exactly as posted",
        "title_source" => "subject | inner_subject (Subject: line inside the message) | body_prefix (cut subject completed from the first line) | headline (first line shaped like a headline) | opening (opening words) | duplicate (title of another copy) | topic",
        "conference_topic" => "IGC conference topic a reply (r) was filed under",
        "outlet_code" => "Outlet code (KMP, JKTP, SIAR ...)",
        "outlet_basis" => "Where the outlet was read: subject, inner_subject, url (X-URL header) or text (opening lines)",
        "message_type" => "news, reply, opinion, statement, rubric, literary or other",
        "duplicate_of" => "For a duplicate copy: the ID of the first copy, which carries the counts",
        "thread_id" => "ID of the first message of the thread (same subject, a reply among them, gaps of at most 45 days)",
        "may1998_tier" => "1 mentions sexual violence, 2 in the same paragraph as the May riots or the inquiries, 3 focused on it (from 13 May 1998)",
        "date_note" => "A caution about the date, when there is one",
        "words" => "Words in the clean text",
        "chars" => "Characters in the clean text",
        "wayback_url" => "Internet Archive capture the text was taken from",
        "original_url" => "URL of the page in the original database",
        _ => "As in the Gerwani Archive Index",
    }
}

struct Ctx {
    parent: String,
    copies: AHashMap<i64, Vec<String>>,
    roots: AHashMap<i64, String>,
    ents: AHashMap<i64, (Vec<String>, Vec<String>, Vec<String>)>,
    themes: AHashMap<i64, Vec<String>>,
}

fn lang_name(l: &str) -> &str {
    match l {
        "id" => "Indonesian",
        "en" => "English",
        "mixed" => "Indonesian; English",
        "und" => "",
        x => x.strip_prefix("other:").unwrap_or(x),
    }
}

fn file_rel(collection: &str, file: &str, date: Option<&str>, ym: Option<&str>) -> String {
    if collection != "basisdata" {
        return format!("02 Apakabar Lists Hypermail Archives (indopubs.com, 1997-2002)/{file}");
    }
    if file.contains("retried captures") {
        return format!("01 Apakabar Messages (1990-2002)/Retried captures/{file}");
    }
    let year = file.get(10..14).filter(|y| y.chars().all(|c| c.is_ascii_digit())).map(|y| y.to_string()).or_else(|| date.or(ym).map(|d| d[..4].to_string())).unwrap_or_default();
    format!("01 Apakabar Messages (1990-2002)/{year}/{file}")
}

fn row(c: &Ctx, r: &rusqlite::Row) -> rusqlite::Result<Vec<String>> {
    let id: i64 = r.get("id")?;
    let ak: String = r.get("ak_id")?;
    let collection: String = r.get("collection")?;
    let date: Option<String> = r.get("date")?;
    let ym: Option<String> = r.get("ym")?;
    let file: String = r.get::<_, Option<String>>("file")?.unwrap_or_default();
    let list_tag: Option<String> = r.get("list_tag")?;
    let title: String = r.get("title")?;
    let sender: String = r.get::<_, Option<String>>("sender")?.unwrap_or_default();
    let wayback: String = r.get::<_, Option<String>>("wayback_url")?.unwrap_or_default();
    let tier: i64 = r.get("may98_tier")?;
    let rel = file_rel(&collection, &file, date.as_deref(), ym.as_deref());
    let (people, orgs, places) = c.ents.get(&id).cloned().unwrap_or_default();
    let themes = c.themes.get(&id).cloned().unwrap_or_default();
    let mut also = Vec::new();
    if tier >= 2 { also.push("May 1998 Sexual Violence Archive"); }
    if orgs.iter().any(|o| o == "Gerwani") { also.push("Gerwani Archive"); }
    let author = sender.split(" (").next().unwrap_or(&sender).to_string();
    let citation = format!(
        "{}. ({}). {} [Mailing-list post, {}]. apakabar (Indonesia Publications, ed. J. A. MacDougall). {}",
        if author.is_empty() { "apakabar".to_string() } else { author.clone() },
        date.as_deref().map(|d| d.to_string()).unwrap_or_else(|| "n.d.".into()),
        title, ak, wayback
    );
    let cap_ts = wayback.split("/web/").nth(1).and_then(|x| x.split('/').next()).unwrap_or("").to_string();
    let code: Option<String> = r.get("source_code")?;
    let dup_of: Option<i64> = r.get("dup_of")?;
    let mut v: AHashMap<&str, String> = AHashMap::new();
    v.insert("item_id", ak.clone());
    v.insert("group_id", format!("AKG-{}", if collection == "basisdata" { ym.clone().unwrap_or_default() } else { crate::parse::list_archive(&ak).unwrap_or_default() }));
    v.insert("related_items", c.copies.get(&dup_of.unwrap_or(id)).map(|xs| xs.iter().filter(|x| **x != ak).cloned().collect::<Vec<_>>().join("; ")).unwrap_or_default());
    v.insert("organisation", "Indonesia Publications".into());
    v.insert("org_code", "IP".into());
    v.insert("also_relevant_to", also.join("; "));
    v.insert("title", title.clone());
    v.insert("date", date.clone().unwrap_or_default());
    v.insert("year", date.as_deref().or(ym.as_deref()).map(|d| d[..4].to_string()).unwrap_or_default());
    v.insert("period", date.as_deref().map(|d| if d < "1998-05-21" { "New Order (to May 1998)" } else { "Reformasi (from 21 May 1998)" }.to_string()).unwrap_or_default());
    v.insert("creator", sender.clone());
    v.insert("publication", format!("apakabar (Indonesia Publications, moderated by John A. MacDougall){}", list_tag.as_deref().map(|l| format!(" / list {l}")).unwrap_or_default()));
    v.insert("document_type", "mailing-list post".into());
    v.insert("document_type_detail", match collection.as_str() { "basisdata" => "apakabar database (Hamline basisdata / Ohio indopubs hypermail)", "iri-l" => "IRI-L list archive at indopubs.com (unrelated to apakabar)", _ => "indopubs.com list hypermail archive" }.into());
    v.insert("language", lang_name(&r.get::<_, String>("lang")?).to_string());
    v.insert("collection", "Apakabar Indonesia Archive".into());
    v.insert("section", rel.split('/').next().unwrap_or("").to_string());
    v.insert("subcollection", if collection == "basisdata" { v["year"].clone() } else { crate::parse::list_archive(&ak).unwrap_or_default() });
    v.insert("file_name", file.clone());
    v.insert("file_path", rel.clone());
    v.insert("file_path_in_parent", format!("{}/{}", c.parent, rel));
    v.insert("file_type", "txt (one file per month, messages separated by ==========)".into());
    v.insert("people", people.join("; "));
    v.insert("organisations", orgs.join("; "));
    v.insert("places", places.join("; "));
    v.insert("topics", themes.join("; "));
    v.insert("source_archive", "Wayback Machine (Internet Archive)".into());
    v.insert("source_url", wayback.clone());
    v.insert("access_rights", "rights unclear (mailing-list reposts; private research copy)".into());
    v.insert("citation_apa", citation);
    v.insert("full_text_available", "yes".into());
    v.insert("text_status", "full text, HTML stripped from hypermail page".into());
    v.insert("text_file", rel.clone());
    v.insert("text_proofread", "no".into());
    v.insert("summary_basis", "catalogue record only (automated)".into());
    v.insert("annotation_level", "automated catalogue".into());
    v.insert("source_role", "primary".into());
    v.insert("country_of_publication", "United States (list hosted in US; content worldwide)".into());
    v.insert("decade", v["year"].get(..3).map(|d| format!("{d}0s")).unwrap_or_default());
    v.insert("date_precision", if date.is_some() { "day" } else if ym.is_some() { "month" } else { "undetermined" }.into());
    v.insert("file_visibility", "private".into());
    v.insert("auto_filled", "yes".into());
    v.insert("list_name", list_tag.unwrap_or_default());
    v.insert("original_url", r.get::<_, Option<String>>("orig_url")?.unwrap_or_default());
    v.insert("wayback_url", wayback);
    v.insert("capture_timestamp", cap_ts);
    v.insert("chars", r.get::<_, i64>("chars")?.to_string());
    v.insert("in_archive", "yes".into());
    v.insert("status", "downloaded".into());
    v.insert("sexual_violence_flag", if r.get::<_, i64>("sv_flag")? == 1 { "yes".into() } else { String::new() });
    v.insert("may1998_flag", if tier >= 2 { "yes".into() } else { String::new() });
    v.insert("subject_as_posted", r.get("subject")?);
    v.insert("title_source", r.get::<_, Option<String>>("title_source")?.unwrap_or_default());
    v.insert("conference_topic", r.get::<_, Option<String>>("topic")?.unwrap_or_default());
    v.insert("outlet", code.as_deref().and_then(|c| sources::BY_CODE.get(c)).map(|s| s.name.to_string()).or(r.get::<_, Option<String>>("source_name")?).unwrap_or_default());
    v.insert("outlet_category", r.get::<_, Option<String>>("source_cat")?.map(|c| sources::category_label(&c).to_string()).unwrap_or_default());
    v.insert("outlet_basis", r.get::<_, Option<String>>("source_basis")?.unwrap_or_default());
    v.insert("outlet_code", code.unwrap_or_default());
    v.insert("message_type", r.get::<_, Option<String>>("msg_type")?.unwrap_or_default());
    v.insert("duplicate_of", dup_of.and_then(|d| c.roots.get(&d).cloned()).unwrap_or_default());
    v.insert("thread_id", r.get::<_, Option<String>>("thread_id")?.unwrap_or_default());
    v.insert("may1998_tier", if tier > 0 { tier.to_string() } else { String::new() });
    v.insert("date_note", r.get::<_, Option<String>>("date_note")?.unwrap_or_default());
    v.insert("words", r.get::<_, i64>("words")?.to_string());
    Ok(COLS.iter().chain(EXTRA).chain(ADDED).map(|k| v.get(k).cloned().unwrap_or_default()).collect())
}

fn header_format() -> Format {
    Format::new().set_bold().set_font_name("Arial").set_font_color(Color::White).set_background_color(Color::RGB(0x7A1F1F))
}

fn write_table(ws: &mut Worksheet, header: &[&str], rows: &[Vec<String>], width: f64) -> std::result::Result<(), rust_xlsxwriter::XlsxError> {
    let h = header_format();
    for (k, name) in header.iter().enumerate() {
        ws.write_string_with_format(0, k as u16, *name, &h)?;
        ws.set_column_width(k as u16, width)?;
    }
    ws.set_freeze_panes(1, 0)?;
    for (n, r) in rows.iter().enumerate() {
        for (k, val) in r.iter().enumerate() {
            if val.is_empty() { continue; }
            match val.parse::<f64>() {
                Ok(x) if !val.starts_with('0') || val == "0" || val.contains('.') => { ws.write_number(n as u32 + 1, k as u16, x)?; }
                _ => { ws.write_string(n as u32 + 1, k as u16, val.chars().take(32000).collect::<String>())?; }
            }
        }
    }
    Ok(())
}

fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn run(db_path: &Path, out: &Path, parent: &str) -> Result<()> {
    std::fs::create_dir_all(out)?;
    let conn = db::open_rw(db_path)?;
    let analysis: AHashMap<String, Value> = conn
        .prepare("SELECT key, json FROM analysis")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .filter_map(|x| x.ok())
        .filter_map(|(k, j)| serde_json::from_str(&j).ok().map(|v| (k, v)))
        .collect();
    // copies, names and themes, loaded once
    let mut copies: AHashMap<i64, Vec<String>> = AHashMap::new();
    let mut roots: AHashMap<i64, String> = AHashMap::new();
    {
        let mut st = conn.prepare("SELECT id, ak_id, dup_of FROM messages WHERE dup_of IS NOT NULL OR id IN (SELECT DISTINCT dup_of FROM messages WHERE dup_of IS NOT NULL)")?;
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<i64>>(2)?)))? {
            let (id, ak, dup) = r?;
            let root = dup.unwrap_or(id);
            if dup.is_none() { roots.insert(id, ak.clone()); }
            copies.entry(root).or_default().push(ak);
        }
    }
    let mut ents: AHashMap<i64, (Vec<String>, Vec<String>, Vec<String>)> = AHashMap::new();
    {
        let mut st = conn.prepare("SELECT me.msg_id, e.name, e.kind FROM message_entities me JOIN entities e ON e.id = me.entity_id ORDER BY me.msg_id, me.hits DESC")?;
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? {
            let (id, name, kind) = r?;
            let e = ents.entry(id).or_default();
            match kind.as_str() { "person" => e.0.push(name), "org" => e.1.push(name), _ => e.2.push(name) }
        }
    }
    let mut themes: AHashMap<i64, Vec<String>> = AHashMap::new();
    {
        let mut st = conn.prepare("SELECT mt.msg_id, t.name FROM message_themes mt JOIN themes t ON t.id = mt.theme_id WHERE mt.about = 1 ORDER BY mt.msg_id, mt.score DESC")?;
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
            let (id, name) = r?;
            themes.entry(id).or_default().push(name);
        }
    }
    let ctx = Ctx { parent: parent.trim_end_matches('/').to_string(), copies, roots, ents, themes };
    let header: Vec<&str> = COLS.iter().chain(EXTRA).chain(ADDED).copied().collect();
    let mut early: Vec<Vec<String>> = Vec::new();
    let mut late: Vec<Vec<String>> = Vec::new();
    let csv_path = out.join("Apakabar Archive Index - Items.csv.gz");
    let mut gz = flate2::write::GzEncoder::new(std::fs::File::create(&csv_path)?, flate2::Compression::new(9));
    writeln!(gz, "{}", header.join(","))?;
    let mut n = 0usize;
    {
        let mut st = conn.prepare("SELECT * FROM messages ORDER BY collection != 'basisdata', coalesce(date, ym || '-00'), ak_id")?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let v = row(&ctx, r)?;
            writeln!(gz, "{}", v.iter().map(|x| csv_field(x)).collect::<Vec<_>>().join(","))?;
            let year: i64 = v[12].parse().unwrap_or(0);
            let is_list = r.get::<_, String>("collection")? != "basisdata";
            if !is_list && year > 0 && year <= 1997 { early.push(v) } else { late.push(v) }
            n += 1;
        }
    }
    gz.finish()?;
    eprintln!("{n} rows; csv written");

    let cov = analysis.get("coverage").cloned().unwrap_or(Value::Null);
    // current check sets below the 90% target, for the README
    let low: Vec<String> = analysis.get("checks").and_then(|c| c["sets"].as_array().cloned()).unwrap_or_default().iter()
        .filter(|c| c["current"].as_bool().unwrap_or(true) && !c["key"].as_str().unwrap_or("").starts_with("theme:"))
        .filter_map(|c| {
            let m = c["results"].as_object()?;
            let (_, r) = m.iter().find(|(k, _)| k.starts_with("user:")).or_else(|| m.iter().next())?;
            let (y, n) = (r["yes"].as_i64().unwrap_or(0), r["no"].as_i64().unwrap_or(0));
            (y + n > 0 && (y as f64) < 0.9 * (y + n) as f64).then(|| format!("{} ({}, {} of {})", c["label"].as_str().unwrap_or(""), c["group"].as_str().unwrap_or(""), y, y + n))
        })
        .collect();
    let readme: Vec<String> = vec![
        "APAKABAR INDONESIA ARCHIVE - INDEX".into(),
        "Catalogue of every recovered message of John A. MacDougall's apakabar lists (October 1990 - February 2002), in the format of the Gerwani Archive Index.".into(),
        format!("Generated from the archive database by `apakabar export` on {}. Rows: {}. Items 1990-1997 are in this workbook; 1998-2002 and the indopubs.com list archives are in \"Apakabar Archive Index - Items 1998-2002 and lists.xlsx\". The gzip-compressed CSV has every row.", chrono::Local::now().format("%Y-%m-%d"), n),
        format!("Coverage: {} of the {} messages the database held ({:.1}%), counted from its own folder listings, which the Wayback Machine kept for {} days; on {} days nothing was filed at all. The {} messages the listings name but no capture holds are in the sheet Lost messages. See also the Coverage sheets.",
            cov["captured"], cov["estimate"], cov["share"].as_f64().unwrap_or(0.0) * 100.0, cov["days"]["listed"], cov["days"]["nofolder"], cov["lost"]),
        "Titles: where the subject line was cut short or only named the IGC conference topic of a reply, the title comes from the text (column title_source); the subject as posted is in subject_as_posted.".into(),
        "Themes (column topics) are the themes a message is about: it uses the theme's words, and a model fitted on hand-checked messages judges the theme a real subject of the text (threshold set for 90% precision). Names (people, organisations, places) come from word lists. May 1998 tiers: 1 mentions sexual violence, 2 speaks of the sexual violence of the May 1998 riots or the inquiries into it, 3 is mainly about it.".into(),
        format!("Precision: every theme, flag, name list and kind of derived title was checked by reading 50 messages drawn at random; the Precision sheet gives each reading (round 1 before the Phase 1 changes, rounds 2 and 3 after them). Below the 90% target: {}.", if low.is_empty() { "none".to_string() } else { low.join("; ") }),
        "IRI-L, an unrelated list found in the indopubs.com archives, is kept apart (document_type_detail) and left out of all figures.".into(),
        "All texts are private research copies (file_visibility = private): most messages repost copyrighted articles and many name private people.".into(),
    ];
    let mut wb = Workbook::new();
    {
        let ws = wb.add_worksheet();
        ws.set_name("README")?;
        ws.set_column_width(0, 160)?;
        for (k, l) in readme.iter().enumerate() {
            ws.write_string(k as u32, 0, l)?;
        }
    }
    {
        let ws = wb.add_worksheet_with_constant_memory()?;
        ws.set_name("Items")?;
        write_table(ws, &header, &early, 18.0)?;
    }
    let num = |v: &Value| v.as_f64().map(|x| if x.fract() == 0.0 { format!("{}", x as i64) } else { format!("{:.3}", x) }).unwrap_or_else(|| v.as_str().unwrap_or("").to_string());
    let months: Vec<Vec<String>> = cov["months"].as_array().cloned().unwrap_or_default().iter().map(|m| vec![num(&m["ym"]), num(&m["captured"]), num(&m["estimate"]), num(&m["share"]), num(&m["days"]), num(&m["days_nofolder"]), num(&m["days_none"])]).collect();
    write_table(wb.add_worksheet().set_name("Coverage by month")?, &["month", "captured", "held_by_database", "share", "days", "days_without_folder", "days_unknown"], &months, 16.0)?;
    let years: Vec<Vec<String>> = cov["years"].as_array().cloned().unwrap_or_default().iter().map(|m| vec![num(&m["year"]), num(&m["captured"]), num(&m["estimate"]), num(&m["share"]), num(&m["days"]), num(&m["days_nofolder"]), num(&m["days_none"])]).collect();
    write_table(wb.add_worksheet().set_name("Coverage by year")?, &["year", "captured", "held_by_database", "share", "days", "days_without_folder", "days_unknown"], &years, 16.0)?;
    let lost: Vec<Vec<String>> = conn
        .prepare("SELECT ak_id, coalesce(date, ''), num, size, title, src, ts, coalesce(maybe, '') FROM lost ORDER BY ak_id")?
        .query_map([], |r| Ok(vec![r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?.to_string(), r.get::<_, String>(3)?, r.get::<_, String>(4)?, crate::listings::source_name(&r.get::<_, String>(5)?).to_string(), r.get::<_, String>(6)?, r.get::<_, String>(7)?]))?
        .collect::<rusqlite::Result<_>>()?;
    write_table(wb.add_worksheet().set_name("Lost messages")?, &["item_id", "date", "number", "size_listed", "title_start", "listing_host", "listing_capture", "possible_copy"], &lost, 20.0)?;
    let coll: Vec<Vec<String>> = conn
        .prepare("SELECT collection, count(*), sum(counted), sum(dup_of IS NOT NULL) FROM messages GROUP BY 1 ORDER BY 2 DESC")?
        .query_map([], |r| Ok(vec![r.get::<_, String>(0)?, r.get::<_, i64>(1)?.to_string(), r.get::<_, i64>(2)?.to_string(), r.get::<_, i64>(3)?.to_string()]))?
        .collect::<rusqlite::Result<_>>()?;
    write_table(wb.add_worksheet().set_name("Collections")?, &["collection", "messages", "counted", "duplicate_copies"], &coll, 18.0)?;
    let topics: Vec<Vec<String>> = analysis.get("topics").and_then(|t| t["topics"].as_array().cloned()).unwrap_or_default().iter().map(|t| vec![num(&t["name"]), num(&t["n"]), num(&t["first"]), num(&t["last"])]).collect();
    write_table(wb.add_worksheet().set_name("Conference topics")?, &["topic", "messages", "first", "last"], &topics, 22.0)?;
    let checks = analysis.get("checks").and_then(|c| c["sets"].as_array().cloned()).unwrap_or_default();
    // precision of a set: the editors' reading when there is one, otherwise the first pass
    let result = |c: &Value| -> Option<(i64, i64, i64, String)> {
        let m = c["results"].as_object()?;
        let (k, r) = m.iter().find(|(k, _)| k.starts_with("user:")).or_else(|| m.iter().next())?;
        Some((r["yes"].as_i64().unwrap_or(0), r["no"].as_i64().unwrap_or(0), r["unsure"].as_i64().unwrap_or(0), if k.starts_with("user:") { "editor".to_string() } else { "first pass".to_string() }))
    };
    // the latest round of a set ("about:aceh/3" before "about:aceh")
    let latest = |base: &str| -> Option<Value> {
        let mut v: Vec<&Value> = checks.iter().filter(|c| c["key"].as_str().map(|k| k.split('/').next() == Some(base)).unwrap_or(false) && result(c).map(|r| r.0 + r.1 > 0).unwrap_or(false)).collect();
        v.sort_by_key(|c| std::cmp::Reverse(c["round"].as_i64().unwrap_or(1)));
        v.first().map(|c| (*c).clone())
    };
    let theme_rows: Vec<Vec<String>> = analysis.get("themes").and_then(|t| t["themes"].as_array().cloned()).unwrap_or_default().iter().map(|t| {
        let prec = latest(&format!("about:{}", t["slug"].as_str().unwrap_or(""))).and_then(|c| result(&c)).map(|(y, n, _, who)| if y + n > 0 { format!("{:.0}% ({} of {}, {})", 100.0 * y as f64 / (y + n) as f64, y, y + n, who) } else { String::new() }).unwrap_or_default();
        let join = |v: &Value| v.as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join("; ")).unwrap_or_default();
        vec![num(&t["name"]), num(&t["total"]), num(&t["mentions"]), num(&t["threshold"]), join(&t["terms"]), join(&t["weak"]), join(&t["context"]), join(&t["stops"]), prec]
    }).collect();
    write_table(wb.add_worksheet().set_name("Themes")?, &["theme", "messages_about", "messages_mentioning", "model_threshold", "terms", "weak_words", "context_words", "ignored_phrases", "precision"], &theme_rows, 30.0)?;
    let prec_rows: Vec<Vec<String>> = checks.iter().map(|c| {
        let (y, n, u, who) = result(c).unwrap_or((0, 0, 0, String::new()));
        vec![num(&c["key"]), num(&c["label"]), num(&c["group"]), num(&c["round"]), if c["current"].as_bool().unwrap_or(true) { "yes".into() } else { "no (checked again)".into() }, num(&c["total"]), (y + n + u).to_string(), y.to_string(), n.to_string(), u.to_string(), if y + n > 0 { format!("{:.3}", y as f64 / (y + n) as f64) } else { String::new() }, who, num(&c["question"])]
    }).collect();
    write_table(wb.add_worksheet().set_name("Precision")?, &["set", "label", "group", "round", "current", "messages_in_set", "read", "yes", "no", "unsure", "precision", "reader", "question"], &prec_rows, 18.0)?;
    let may: Vec<Vec<String>> = conn
        .prepare("SELECT ak_id, date, may98_tier, title, coalesce(source_code,''), coalesce(wayback_url,'') FROM messages WHERE counted = 1 AND may98_tier >= 2 ORDER BY date, ak_id")?
        .query_map([], |r| Ok(vec![r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get::<_, i64>(2)?.to_string(), r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?]))?
        .collect::<rusqlite::Result<_>>()?;
    write_table(wb.add_worksheet().set_name("May 1998 tiers 2-3")?, &["item_id", "date", "tier", "title", "outlet_code", "wayback_url"], &may, 20.0)?;
    let srcs: Vec<Vec<String>> = conn
        .prepare("SELECT source_code, max(source_name), max(source_cat), count(*), min(year), max(year) FROM messages WHERE counted = 1 AND source_code IS NOT NULL GROUP BY 1 ORDER BY 4 DESC")?
        .query_map([], |r| Ok(vec![r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get::<_, Option<String>>(2)?.map(|c| sources::category_label(&c).to_string()).unwrap_or_default(), r.get::<_, i64>(3)?.to_string(), r.get::<_, Option<i64>>(4)?.map(|x| x.to_string()).unwrap_or_default(), r.get::<_, Option<i64>>(5)?.map(|x| x.to_string()).unwrap_or_default()]))?
        .collect::<rusqlite::Result<_>>()?;
    write_table(wb.add_worksheet().set_name("Outlets")?, &["code", "outlet", "category", "messages", "first_year", "last_year"], &srcs, 20.0)?;
    let dd: Vec<Vec<String>> = header.iter().map(|c| vec![c.to_string(), dictionary(c).to_string()]).collect();
    write_table(wb.add_worksheet().set_name("Data dictionary")?, &["column", "meaning"], &dd, 60.0)?;
    wb.save(out.join("Apakabar Archive Index.xlsx"))?;
    eprintln!("workbook 1 written ({} items)", early.len());

    let mut wb2 = Workbook::new();
    {
        let ws = wb2.add_worksheet();
        ws.set_name("README")?;
        ws.set_column_width(0, 160)?;
        ws.write_string(0, 0, "Items 1998-2002 and the indopubs.com list archives. Continuation of the Items sheet in \"Apakabar Archive Index.xlsx\" (split to keep each file small). Same columns; see the README and Data dictionary there.")?;
    }
    {
        let ws = wb2.add_worksheet_with_constant_memory()?;
        ws.set_name("Items")?;
        write_table(ws, &header, &late, 18.0)?;
    }
    wb2.save(out.join("Apakabar Archive Index - Items 1998-2002 and lists.xlsx"))?;
    eprintln!("workbook 2 written ({} items)", late.len());
    let _ = analyze::check_sets;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_and_fields() {
        assert_eq!(file_rel("basisdata", "apakabar (1998-05) - 3180 messages.txt", Some("1998-05-14"), None), "01 Apakabar Messages (1990-2002)/1998/apakabar (1998-05) - 3180 messages.txt");
        assert_eq!(file_rel("basisdata", "apakabar (retried captures, 1990-1998) - 30 messages.txt", Some("1994-01-01"), None), "01 Apakabar Messages (1990-2002)/Retried captures/apakabar (retried captures, 1990-1998) - 30 messages.txt");
        assert_eq!(csv_field("a, \"b\""), "\"a, \"\"b\"\"\"");
        assert_eq!(COLS.len() + EXTRA.len(), 87);
    }
}
