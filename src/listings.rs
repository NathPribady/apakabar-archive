//! The database's own folder listings.
//!
//! The basisdata kept every message as a file in a folder per day (`1998/05/15/0003.html`), and
//! the web servers that hosted it (Hamline University, Ohio University Library, the esosoft
//! mirror) showed each folder as an Apache "Index of" page: the files in it, their sizes and the
//! first 23 characters of each title. The Wayback Machine captured those pages for most days, and
//! month pages that list the day folders. With them the coverage can be counted exactly, the days
//! on which nothing was filed told apart from days that were lost, and the messages that no
//! capture holds named by date, number, size and the first words of their titles.
//!
//! The listings are read from JSON files in `00 Index and Working Files/_cache/listings/`
//! (written by the harvest in October 2026): `{"days": {"YYYYMMDD": {"src", "ts", "msgs":
//! [[number, size, description], ...]}}, "months": {"YYYYMM": {"src", "ts", "dirs": ["01", ...],
//! "msgs": [...]}}}`. Several files are merged; a later file adds to or replaces an earlier one.

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct Listing {
    /// (number, size as listed, description = the start of the title)
    pub files: Vec<(u32, String, String)>,
    /// day folders, for a month listing ("01".."31")
    pub dirs: Vec<String>,
    /// h = Hamline, e = esosoft, o = Ohio University
    pub src: String,
    /// Wayback capture timestamp of the listing page
    pub ts: String,
}

#[derive(Debug, Default)]
pub struct Listings {
    /// "YYYY-MM-DD"
    pub days: BTreeMap<String, Listing>,
    /// "YYYY-MM"
    pub months: BTreeMap<String, Listing>,
}

pub fn source_name(src: &str) -> &'static str {
    match src {
        "h" => "Hamline University",
        "e" => "basisdata.esosoft.net",
        "o" => "Ohio University Library",
        _ => "unknown",
    }
}

fn entry(v: &Value) -> Listing {
    let files = v["msgs"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|f| {
                    let n = f.get(0)?.as_str()?.parse::<u32>().ok()?;
                    let size = f.get(1).and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let desc = f.get(2).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                    Some((n, size, desc))
                })
                .collect()
        })
        .unwrap_or_default();
    let dirs = v["dirs"].as_array().map(|a| a.iter().filter_map(|d| d.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
    Listing { files, dirs, src: v["src"].as_str().unwrap_or("").to_string(), ts: v["ts"].as_str().unwrap_or("").to_string() }
}

/// Merge one listings file (already parsed) into `out`.
pub fn merge(out: &mut Listings, v: &Value) {
    if let Some(days) = v["days"].as_object() {
        for (k, d) in days {
            if k.len() == 8 && k.chars().all(|c| c.is_ascii_digit()) {
                out.days.insert(format!("{}-{}-{}", &k[..4], &k[4..6], &k[6..]), entry(d));
            }
        }
    }
    if let Some(months) = v["months"].as_object() {
        for (k, m) in months {
            if k.len() == 6 && k.chars().all(|c| c.is_ascii_digit()) {
                let mut l = entry(m);
                l.dirs.sort();
                l.dirs.dedup();
                out.months.insert(format!("{}-{}", &k[..4], &k[4..]), l);
            }
        }
    }
}

/// Read every listings file of the archive folder. A missing folder is not an error: the
/// coverage is then estimated from the message numbers alone.
pub fn load(input: &Path) -> Listings {
    let mut out = Listings::default();
    let dir = input.join("00 Index and Working Files").join("_cache").join("listings");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x == "json").unwrap_or(false)).collect())
        .unwrap_or_default();
    files.sort();
    for f in files {
        match std::fs::read(&f).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
            Some(v) => merge(&mut out, &v),
            None => eprintln!("note: could not read {}", f.display()),
        }
    }
    out
}

/// The AK IDs a listing names: day folders give AK-YYYY-MM-DD-NNNN, the month folder of early
/// 2000 gives AK-YYYY-MM-NNNN.
pub fn listed_ids(l: &Listings) -> Vec<(String, String, u32, &str, &str, &str, &str)> {
    let mut v = Vec::new();
    for (day, d) in &l.days {
        for (n, size, desc) in &d.files {
            v.push((format!("AK-{day}-{n:04}"), day.clone(), *n, size.as_str(), desc.as_str(), d.src.as_str(), d.ts.as_str()));
        }
    }
    for (ym, m) in &l.months {
        for (n, size, desc) in &m.files {
            v.push((format!("AK-{ym}-{n:04}"), ym.clone(), *n, size.as_str(), desc.as_str(), m.src.as_str(), m.ts.as_str()));
        }
    }
    v
}

/// Lowercase letters and digits of a title start, without a list tag ("[INDONESIA-NEWS] KMP ->"
/// -> "kmp") or the "<list> Archives:" prefix of the indopubs.com copies; used to look for a
/// surviving copy of a lost message among the list archives.
pub fn title_key(s: &str) -> String {
    let mut t = s.trim().trim_end_matches('>').trim();
    if let Some(p) = t.find(" Archives:") {
        if !t[..p].contains(' ') {
            t = t[p + 10..].trim_start();
        }
    }
    if t.starts_with('[') {
        if let Some(e) = t.find(']') {
            t = t[e + 1..].trim_start();
        }
    }
    t.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { ' ' }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merge_and_ids() {
        let v: Value = serde_json::from_str(r#"{"days": {"19980515": {"src": "e", "ts": "20020722200008", "msgs": [["0000", "6k", "[INDONESIA-L] KOLOM - >"], ["0001", "5k", "Hard Line in Timor"]]}},
            "months": {"199805": {"src": "e", "ts": "20020422233243", "dirs": ["15", "13", "13"], "msgs": []}, "200001": {"src": "h", "ts": "1", "dirs": [], "msgs": [["2571", "3k", "x"]]}}}"#).unwrap();
        let mut l = Listings::default();
        merge(&mut l, &v);
        assert_eq!(l.days["1998-05-15"].files.len(), 2);
        assert_eq!(l.months["1998-05"].dirs, vec!["13", "15"]);
        let ids: Vec<String> = listed_ids(&l).into_iter().map(|x| x.0).collect();
        assert!(ids.contains(&"AK-1998-05-15-0001".to_string()) && ids.contains(&"AK-2000-01-2571".to_string()));
        assert_eq!(title_key("[INDONESIA-NEWS] KMP ->"), "kmp");
        assert_eq!(title_key("indonesia-news Archives: [INDONESIA-NEWS] KMP - Harga beras"), "kmp harga beras");
        assert_eq!(title_key("Deported Japanese Spea>"), "deported japanese spea");
    }
}
