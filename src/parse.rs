//! Reads the harvested text files ("========== [KEY] subject" blocks) into raw records.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct RawMsg {
    /// AK-YYYY-MM-DD-NNNN for the basisdata (day folders), AK-YYYY-MM-NNNN for the month
    /// folder of January-March 2000, AKL-<archive>-NNNN for the indopubs.com lists
    pub ak_id: String,
    #[allow(dead_code)]
    pub collection: Collection,
    pub subject: String,
    pub from: String,
    pub date_line: String,
    pub orig_url: String,
    pub wayback_url: String,
    pub body: String,
    pub file: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Collection {
    Basisdata,
    List,
}

impl Collection {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Collection::Basisdata => "basisdata",
            Collection::List => "list",
        }
    }
}

/// All harvested .txt files below `root` (both collections), sorted.
pub fn find_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name.starts_with('_') || name.starts_with("00 ") {
                        continue;
                    }
                    stack.push(p);
                } else if p.extension().map(|x| x == "txt").unwrap_or(false) {
                    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name.starts_with("apakabar (") || name.starts_with("indopubs list ") {
                        out.push(p);
                    }
                }
            }
        }
    }
    out.sort();
    out
}

/// Parse one harvested file. Failed captures ("(capture failed to load)") are skipped:
/// their text, where it was recovered, lives in the "retried captures" files.
pub fn parse_file(path: &Path) -> Vec<RawMsg> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(_) => return vec![],
    };
    let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
    let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
    let is_list = fname.starts_with("indopubs list");
    let mut out = Vec::new();
    // split on the block marker at line start
    let marker = "\n========== [";
    let mut idx: Vec<usize> = text.match_indices(marker).map(|(i, _)| i + 1).collect();
    if text.starts_with("========== [") {
        idx.insert(0, 0);
    }
    for (n, &start) in idx.iter().enumerate() {
        let end = if n + 1 < idx.len() { idx[n + 1] } else { text.len() };
        let block = &text[start..end];
        let first_nl = block.find('\n').unwrap_or(block.len());
        let head = &block[..first_nl];
        // head: ========== [KEY] subject
        let k0 = match head.find('[') { Some(i) => i + 1, None => continue };
        let k1 = match head[k0..].find(']') { Some(i) => k0 + i, None => continue };
        let key = head[k0..k1].trim().to_string();
        let subject = head[k1 + 1..].trim().to_string();
        if subject.starts_with("(capture failed") {
            continue;
        }
        let rest = &block[first_nl.min(block.len())..];
        let rest = rest.strip_prefix('\n').unwrap_or(rest);
        let mut from = String::new();
        let mut date_line = String::new();
        let mut orig_url = String::new();
        let mut wayback = String::new();
        let mut body_start = 0usize;
        let mut pos = 0usize;
        for line in rest.split_inclusive('\n') {
            let l = line.trim_end_matches('\n');
            pos += line.len();
            if let Some(v) = l.strip_prefix("From: ") {
                from = v.trim().to_string();
            } else if let Some(v) = l.strip_prefix("Date: ") {
                date_line = v.trim().to_string();
            } else if let Some(v) = l.strip_prefix("Source: ") {
                orig_url = v.trim().to_string();
            } else if let Some(v) = l.strip_prefix("Wayback: ") {
                wayback = v.trim().to_string();
            } else if l.trim().is_empty() {
                body_start = pos;
                break;
            } else {
                body_start = pos - line.len();
                break;
            }
        }
        let body = rest[body_start.min(rest.len())..].trim_end().to_string();
        let ak_id = if key.starts_with("AK-") {
            key.clone()
        } else {
            format!("AKL-{}", key.replace('/', "-"))
        };
        out.push(RawMsg {
            ak_id,
            collection: if is_list { Collection::List } else { Collection::Basisdata },
            subject,
            from,
            date_line,
            orig_url,
            wayback_url: wayback,
            body,
            file: fname.clone(),
        });
    }
    out
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// The three ID forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdKind {
    /// AK-YYYY-MM-DD-NNNN: a message in a day folder of the database
    Day,
    /// AK-YYYY-MM-NNNN: a message in the month folder used from 4 January to 11 March 2000
    Month,
    /// AKL-<archive>-NNNN: the indopubs.com list archives
    List,
}

pub fn id_kind(ak: &str) -> IdKind {
    if ak.starts_with("AKL-") {
        return IdKind::List;
    }
    let parts: Vec<&str> = ak.split('-').collect();
    if parts.len() == 4 { IdKind::Month } else { IdKind::Day }
}

/// The day in a day-folder ID.
pub fn id_date(ak: &str) -> Option<String> {
    if id_kind(ak) != IdKind::Day {
        return None;
    }
    let rest = ak.strip_prefix("AK-")?;
    if rest.len() >= 10 && rest.as_bytes()[4] == b'-' && rest.as_bytes()[7] == b'-' {
        Some(rest[..10].to_string())
    } else {
        None
    }
}

/// The month in any database ID (day or month folder).
pub fn id_month(ak: &str) -> Option<String> {
    let rest = ak.strip_prefix("AK-")?;
    if rest.len() >= 7 && rest.as_bytes()[4] == b'-' { Some(rest[..7].to_string()) } else { None }
}

/// The message number within its folder (day, month or list archive).
pub fn id_number(ak: &str) -> Option<u32> {
    ak.rsplit('-').next().and_then(|n| n.parse().ok())
}

/// The list archive of an AKL ID ("AKL-iri-l-archives-0489" -> "iri-l-archives").
pub fn list_archive(ak: &str) -> Option<String> {
    let rest = ak.strip_prefix("AKL-")?;
    rest.rfind('-').map(|i| rest[..i].to_string())
}

/// ISO date (YYYY-MM-DD) from a date line: hypermail ("Fri May 01 1998 - 13:44:00 EDT"),
/// RFC 822 ("Thu, 30 Apr 1998 18:19:15 +0700") or ISO.
pub fn date_from_line(date_line: &str) -> Option<String> {
    let parts: Vec<&str> = date_line.split_whitespace().collect();
    for w in parts.windows(3) {
        if let Some(m) = MONTHS.iter().position(|x| *x == w[0].trim_end_matches(',')) {
            if let (Ok(d), Ok(y)) = (w[1].trim_end_matches(',').parse::<u32>(), w[2].trim_end_matches(',').parse::<u32>()) {
                if (1..=31).contains(&d) && (1980..=2010).contains(&y) {
                    return Some(format!("{:04}-{:02}-{:02}", y, m + 1, d));
                }
            }
        }
    }
    for w in parts.windows(3) {
        if let Some(m) = MONTHS.iter().position(|x| *x == w[1]) {
            if let (Ok(d), Ok(y)) = (w[0].trim_end_matches(',').parse::<u32>(), w[2].trim_end_matches(',').parse::<u32>()) {
                if (1..=31).contains(&d) && (1980..=2010).contains(&y) {
                    return Some(format!("{:04}-{:02}-{:02}", y, m + 1, d));
                }
            }
        }
    }
    let t = date_line.trim();
    if t.len() >= 10 && t.as_bytes()[4] == b'-' && t.as_bytes()[7] == b'-' && t[..4].parse::<u32>().map(|y| (1980..=2010).contains(&y)).unwrap_or(false) {
        return Some(t[..10].to_string());
    }
    None
}

/// Date from a date line, falling back on the day in a day-folder ID.
#[allow(dead_code)]
pub fn iso_date(date_line: &str, ak_id: &str) -> Option<String> {
    date_from_line(date_line).or_else(|| id_date(ak_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_and_dates() {
        assert_eq!(id_kind("AK-1998-05-14-0003"), IdKind::Day);
        assert_eq!(id_kind("AK-2000-01-2571"), IdKind::Month);
        assert_eq!(id_kind("AKL-iri-l-archives-0489"), IdKind::List);
        assert_eq!(id_date("AK-1998-05-14-0003").as_deref(), Some("1998-05-14"));
        assert_eq!(id_date("AK-2000-01-2571"), None);
        assert_eq!(id_month("AK-2000-01-2571").as_deref(), Some("2000-01"));
        assert_eq!(id_number("AKL-iri-l-archives-0489"), Some(489));
        assert_eq!(list_archive("AKL-iri-l-archives-0489").as_deref(), Some("iri-l-archives"));
        assert_eq!(date_from_line("Fri May 01 1998 - 13:44:00 EDT").as_deref(), Some("1998-05-01"));
        assert_eq!(date_from_line("Thu, 30 Apr 1998 18:19:15 +0700").as_deref(), Some("1998-04-30"));
        assert_eq!(iso_date("", "AK-2000-01-2571"), None);
        assert_eq!(iso_date("", "AK-1999-02-03-0001").as_deref(), Some("1999-02-03"));
    }
}
