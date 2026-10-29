//! Real titles for messages whose subject line is not one.
//!
//! Two problems come from the way the database was fed:
//!
//! - On the IGC conference behind the database, every reply ("(r)") carries the name of the
//!   topic it was filed under ("Law & Liberty", "Religion", "Area Studies: East Timor"), not
//!   a subject of its own. 10,816 messages had such a topic name as their title.
//! - The conference software cut subjects at about 20 to 50 characters ("Diskusi Agam",
//!   "WNI Keturuna").
//!
//! The real headline is usually in the first lines of the text. The rules, in order:
//!
//! 1. a Subject: line inside the message (forwarded mail keeps it whole);
//! 2. a line near the top that begins with the cut subject and continues it;
//! 3. for topic replies, the first line that is shaped like a headline;
//! 4. for topic replies without one, the opening words of the text.
//!
//! Each derived title records which rule produced it (`title_source`), and the subject as
//! posted is always kept beside it.

use once_cell::sync::Lazy;
use regex::Regex;

#[derive(Debug, Clone, PartialEq)]
pub struct Derived {
    pub title: String,
    pub source: &'static str,
}

static SEPARATOR: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[-=_*~#.+<>:|/\\ ]{3,}$").unwrap());
static HEADERISH: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(date|from|to|cc|bcc|subject|subjec|sender|sent|received|message-id|reply-to|return-path|organi[sz]ation|x-[a-z-]+|forwarded|-{2,}|begin forwarded|original message|kepada|perihal|tanggal|hal)\s*[:\-]").unwrap()
});
static NOISE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(\[.*\]|\(.*\)|https?://\S+|www\.\S+|<https?://.*>|indonesia-[a-z]+|iri-l|apakabar|distributed (via|by)|posted (by|to)|copyright|\(c\)|all rights reserved|source\s*:|sumber\s*:|e-?mail\s*:|tel(p|epon)?\s*[:.]|fax\s*[:.]|return to\b|text-only|home page|homepage|click here|untuk berlangganan|to subscribe|unsubscribe|forwarded by|forwarded from|reposted|dikirim oleh|received from|(brief )?excerpts? from|this is a translation|berikut ini|below is|the following|remark\s*:|note\s*:|catatan\s*:|n\.?b\.?\s*:|forwarded courtesy|courtesy of|selamat membaca|extract(ed)? from|excerpted from|original poster wrote|\S+(\s+\S+)?\s+wrote\s*:?\s*$|\S+(\s+\S+)?\s+menulis\s*:?\s*$|english translation of|translated from|abridged|g'day|continuation of|kutipan dari|readers kindly note|anon(ymous)? posting|diisi tanggal|title\s*:|someone\s*:|para netter|netters\b|saudara\s|kolom\b|rubrik\b|appendix\b|lampiran\b|correction\b|ralat\b|koreksi\b)").unwrap()
});
/// Lines that are not titles: a contributor's signature line ("Dari Ratuadil", "From Krisna"),
/// greetings, invocations and salutations.
static NOT_TITLE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^((dari|from|oleh|by)\s+[\w.'-]+(\s+[\w.'-]+){0,2}\s*[:.\-]?|merdeka+\s*!*|bismillah\w*|assalam\w*|salam\b.*|dear\b.*|yth\b.*|kepada yth.*|hi\b.*|hallo\b.*|halo\b.*|rekan-rekan\b.*|saudara-saudara\b.*|bung\s+\w+\s*,?|pak\s+\w+\s*,?|om\s+\w+\s*,?|mas\s+\w+\s*,?|green left issue\b.*|issue \d+.*|response \d+ of \d+.*|[\w.'-]+ and others\b.*|[\w.'-]+ dan kawan-kawan\b.*)$").unwrap()
});
static BYLINE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(oleh|by|penulis|written by|reported by|laporan|reporter|wartawan|kontributor|koresponden|correspondent|editor|redaksi|disusun oleh|ditulis oleh)\s*[:\-]?\s+\S").unwrap()
});
static MONTH: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(jan(uari|uary)?|feb(ruari|ruary)?|mar(et|ch)?|apr(il)?|mei|may|jun[ei]?|jul[iy]?|ag(ustus)?|aug(ust)?|sep(t(ember)?)?|okt(ober)?|oct(ober)?|nop(ember)?|nov(ember)?|des(ember)?|dec(ember)?|senin|selasa|rabu|kamis|jumat|jum'at|sabtu|minggu|monday|tuesday|wednesday|thursday|friday|saturday|sunday)\b").unwrap()
});
static OUTLET: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(kompas|republika|jakarta post|tempo|gatra|reuters?|afp|associated press|new straits times|straits times|sydney morning herald|the age|media indonesia|suara pembaruan|suara merdeka|jawa pos|pikiran rakyat|detik(\.com)?|siar|xpos|antara|forum keadilan|d&r|bbc|voa|voice of america|radio nederland|radio australia|abc|asiaweek|far eastern economic review|feer|economist|guardian|green left|tapol|amnesty|human rights watch|waspada|bisnis indonesia|indonesian observer|business times|south china morning post|new york times|washington post|asian wall street journal|wall street journal|kyodo|upi|ips|merdeka|rakyat merdeka|sinar harapan|kontan|ummat|panji masyarakat|adil|editor|tajuk|kabar dari pijar|pijar|astaga|satunet|kompas online|kompas cyber media|los angeles times|business america|bulletin of indonesian economic studies)\b").unwrap()
});
static OLEH_TAIL: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+(?:Oleh|OLEH)\s*:?\s+\S.*$").unwrap());
/// Topic lines of the IGC, GreenNet and Pegasus conferences forwarded into apakabar:
/// "Topic 353    Timor Event in NYC 5/12    Response 3 of 3  cscheiner  reg.easttimor ..."
static CONF_TOPIC: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"Topic\s+\d{1,4}\s{2,}(\S.*?)(?:\s*Response\s+\d+\s+of\s+\d+.*|\s{3,}.*|\s+/\s.*)?$").unwrap()
});
/// The same line from the 1991 IGC pages, where the cells ran together:
/// "Topic 42SierraClub Int'l Alert Network! econeten.announcemen 9:36 pm  Jan  7, 1991"
static CONF_TOPIC_GLUED: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\s*Topic\s*\d{1,4}(\S.*?)(?:\s*Response\s+\d+\s+of\s+\d+.*|\s+(?:\S+\s+)?\S*?\d{1,2}:\d{2}\s*[ap]m\b.*)?$").unwrap()
});
static TOPIC_LINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*Topic\s*\d{1,4}").unwrap());
static TRAILING_NUM: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*\(?\d{1,2}\)?$").unwrap());

/// Lowercase letters and digits with single spaces: the form in which titles are compared.
pub fn norm(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_alphanumeric() {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        } else {
            space = true;
        }
    }
    out
}

fn letters_upper_share(s: &str) -> f64 {
    let (mut up, mut all) = (0usize, 0usize);
    for c in s.chars().filter(|c| c.is_alphabetic()) {
        all += 1;
        if c.is_uppercase() {
            up += 1;
        }
    }
    if all == 0 { 0.0 } else { up as f64 / all as f64 }
}

const ACRONYMS: &[&str] = &[
    "ABRI", "TNI", "POLRI", "DPR", "DPRD", "MPR", "MPRS", "PKI", "PDI", "PDI-P", "PDIP", "PPP", "PAN", "PKB", "PBB", "PRD", "PNI", "AJI", "LBH", "YLBHI",
    "KKN", "IMF", "HAM", "RI", "AS", "UN", "US", "USA", "UK", "EU", "ASEAN", "APEC", "NGO", "LSM", "NU", "ICMI", "SBSI", "KPU", "GAM", "OPM", "CNRT",
    "TGPF", "ELSAM", "WALHI", "BUMN", "BI", "BULOG", "IPTN", "CSIS", "LIPI", "UI", "UGM", "ITB", "IPB", "DKI", "SARA", "SIUPP", "SIUP", "BAKIN", "BAIS",
    "G30S", "HMI", "GMNI", "PMII", "PMKRI", "GMKI", "UNTAET", "INTERFET", "UNAMET", "AFP", "AP", "UPI", "BBC", "VOA", "ABC", "SMH", "NST", "RCTI",
    "SCTV", "TVRI", "RRI", "PBNU", "KNPI", "SPSI", "FBSI", "IAIN", "MUI", "TII", "NII", "PIJAR", "SIAR", "XPOS", "KDP", "D&R", "TKI",
    "TKW", "BJ", "SBY", "FPI", "KAMMI", "HTI", "PK", "PKS", "PDKB", "PUDI", "PBR", "II", "III", "IV", "VI", "VII", "VIII", "IX", "XI",
    "XII", "UUD", "UU", "RUU", "PP", "SK", "KUHP", "KUHAP", "BPK", "BPKP", "KPK", "KPKPN", "MK", "BIA", "TPF", "KOMNAS", "HAMI", "JSX", "DI/TII", "NII/TII",
    "ETAN", "TAPOL", "HRW", "ASIA", "RCTI", "IMET", "USAID", "NATO", "CIA", "FBI", "PRC", "ROC", "PNG", "OPEC", "WTO",
];
const LOWER_WORDS: &[&str] = &[
    "dan", "di", "ke", "dari", "yang", "untuk", "dengan", "pada", "dalam", "atau", "oleh", "atas", "bagi", "kepada", "akan", "itu", "ini", "pun",
    "the", "of", "and", "in", "on", "to", "for", "a", "an", "at", "by", "or", "as", "with", "from", "is", "vs",
];

/// "SUHARTO HARUS REALISTIK: DEMOKRATISASI" becomes "Suharto Harus Realistik: Demokratisasi";
/// acronyms (ABRI, PKI, DPR, G30S) keep their capitals. Titles in mixed case are left alone.
pub fn smart_case(s: &str) -> String {
    let letters = s.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 4 || letters_upper_share(s) < 0.9 {
        return s.to_string();
    }
    let mut out: Vec<String> = Vec::new();
    for (i, w) in s.split(' ').enumerate() {
        let core: String = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '&' && c != '-').to_string();
        let lead_len = w.find(|c: char| c.is_alphanumeric()).unwrap_or(0);
        if core.is_empty() {
            out.push(w.to_string());
            continue;
        }
        let fixed = core.split('/').map(|part| fix_word(part, i)).collect::<Vec<_>>().join("/");
        let mut word = String::new();
        word.push_str(&w[..lead_len]);
        word.push_str(&fixed);
        let tail_start = lead_len + core.len();
        if tail_start <= w.len() {
            word.push_str(&w[tail_start..]);
        }
        out.push(word);
    }
    out.join(" ")
}

fn fix_word(core: &str, i: usize) -> String {
    let up = core.to_uppercase();
    let lower = core.to_lowercase();
    if ACRONYMS.contains(&up.as_str()) || core.chars().any(|c| c.is_ascii_digit()) {
        up
    } else if i > 0 && LOWER_WORDS.contains(&lower.as_str()) {
        lower
    } else {
        lower.split('-').map(cap_first).collect::<Vec<_>>().join("-")
    }
}

fn cap_first(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn clean_title(l: &str) -> String {
    let mut t = l.split_whitespace().collect::<Vec<_>>().join(" ");
    t = OLEH_TAIL.replace(&t, "").into_owned();
    let t = t.trim_matches(|c: char| c == '"' || c == '\'' || c == '*' || c == '\u{201c}' || c == '\u{201d}' || c == ' ');
    let t = t.trim_end_matches(|c: char| c == ':' || c == '-' || c == '\u{2013}' || c == '|' || c == ' ' || c == ',' || c == ';');
    let mut t = smart_case(t);
    if t.chars().count() > 200 {
        t = cut_words(&t, 190);
    }
    t
}

fn cut_words(s: &str, max: usize) -> String {
    let mut out = String::new();
    for w in s.split_whitespace() {
        if out.chars().count() + w.chars().count() + 1 > max {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out.push('\u{2026}');
    out
}

/// Citations of periodical issues ("v 21 n 8") and the time stamps of conference postings
/// ("reg.indonesia 1:16 pm Aug 29, 1991").
static CITATION: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\bv(ol)?\.?\s*\d+\s*,?\s*(n|no)\.?\s*\d+\b").unwrap());
static CONF_STAMP: Lazy<Regex> = Lazy::new(|| Regex::new(r"\d{1,2}:\d{2}\s*[ap]m\s+[A-Z][a-z]{2}\s+\d{1,2},\s+\d{4}").unwrap());

fn is_noise(l: &str) -> bool {
    let t = l.trim();
    t.is_empty()
        || t.starts_with('>')
        || t.starts_with('|')
        || t.contains(">>")
        || CITATION.is_match(t)
        || CONF_STAMP.is_match(t)
        || SEPARATOR.is_match(t)
        || HEADERISH.is_match(t)
        || NOISE.is_match(t)
        || (t.contains('@') && t.split_whitespace().count() <= 6)
        || !t.chars().any(|c| c.is_alphabetic())
}

fn is_byline(l: &str) -> bool {
    BYLINE.is_match(l.trim()) || NOT_TITLE.is_match(l.trim())
}

fn is_dateline(l: &str) -> bool {
    let t = l.trim();
    t.chars().count() <= 60 && t.split_whitespace().count() <= 9 && MONTH.is_match(t) && t.chars().any(|c| c.is_ascii_digit())
}

fn is_outlet_line(l: &str) -> bool {
    let t = l.trim();
    (t.chars().count() <= 60 && t.split_whitespace().count() <= 7 && OUTLET.is_match(t)) || (t.chars().count() <= 50 && t.ends_with(']') && t.contains('['))
}

/// Is lead[i] shaped like a headline? `lead` holds the first lines of the text, with a
/// single "" where blank lines were.
fn headline_shape(lead: &[String], i: usize, prefix_matched: bool) -> bool {
    let l = lead[i].trim();
    let n = l.chars().count();
    let words = l.split_whitespace().count();
    if n < 4 || n > 160 || words > 24 || l.ends_with(',') || l.ends_with(';') || !l.chars().any(|c| c.is_alphabetic()) {
        return false;
    }
    let next = lead.get(i + 1).map(|s| s.trim());
    let next_blank = next.map(|x| x.is_empty()).unwrap_or(true);
    let next_lower = next.and_then(|x| x.chars().next()).map(|c| c.is_lowercase()).unwrap_or(false);
    let caps = letters_upper_share(l) >= 0.9 && l.chars().filter(|c| c.is_alphabetic()).count() >= 4;
    let next_meta = next.map(|x| is_byline(x) || is_dateline(x) || is_outlet_line(x)).unwrap_or(false);
    if prefix_matched {
        return next_blank || caps || next_meta || (n <= 100 && !next_lower);
    }
    if n < 6 {
        return false;
    }
    if caps && n <= 130 {
        return true;
    }
    // a wrapped line of prose: long, many words, starts in lower case or stops mid-sentence
    let first_lower = l.chars().find(|c| c.is_alphabetic()).map(|c| c.is_lowercase()).unwrap_or(false);
    let last = l.chars().last().unwrap_or(' ');
    if first_lower || (words >= 10 && n >= 60 && last.is_lowercase()) {
        return false;
    }
    if next_meta && n <= 120 {
        return true;
    }
    next_blank && (n <= 80 || (n <= 120 && !l.ends_with('.')))
}

/// True when the full stop at `k` closes an abbreviation or an initial rather than a sentence.
fn abbreviation_before(s: &str, k: usize) -> bool {
    if s.as_bytes().get(k) != Some(&b'.') {
        return false;
    }
    let word: String = s[..k].chars().rev().take_while(|c| !c.is_whitespace()).collect::<Vec<_>>().into_iter().rev().collect();
    let w = word.trim_start_matches(|c: char| !c.is_alphanumeric());
    let letters = w.chars().filter(|c| c.is_alphabetic()).count();
    (letters == 1 && w.chars().all(|c| c.is_alphabetic() || c == '.'))
        || w.contains('.')
        || matches!(w.to_lowercase().as_str(), "mr" | "mrs" | "ms" | "dr" | "prof" | "no" | "st" | "jl" | "gen" | "lt" | "col" | "capt" | "brig" | "maj" | "sgt" | "rev" | "ir" | "drs" | "hj" | "kol" | "jend" | "pdt" | "sdr" | "bpk" | "dll" | "dsb" | "vs" | "etc" | "inc" | "co" | "corp" | "ltd" | "jr" | "sr")
}

/// The first sentence of the first prose paragraph, shortened to about 100 characters.
pub fn opening(text: &str) -> Option<String> {
    for p in text.split("\n\n").take(8) {
        let p = p.trim();
        let first_line = p.lines().next().unwrap_or("");
        if p.starts_with('>') || p.split_whitespace().count() < 6 || is_noise(first_line) || is_byline(first_line) || p.starts_with("/*") || TOPIC_LINE.is_match(first_line) {
            continue;
        }
        let flat = p.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut end = None;
        for (k, _) in flat.match_indices(|c| c == '.' || c == '?' || c == '!') {
            if k >= 25 && !abbreviation_before(&flat, k) {
                let after = flat[k + 1..].chars().next();
                if after.map(|c| c == ' ').unwrap_or(true) {
                    end = Some(k + 1);
                    break;
                }
            }
        }
        let sentence = match end {
            Some(e) => flat[..e].to_string(),
            None => flat.clone(),
        };
        return Some(if sentence.chars().count() > 150 { cut_words(&sentence, 140) } else { sentence });
    }
    None
}

fn usable(t: &str) -> bool {
    let n = norm(t);
    n.chars().filter(|c| c.is_alphabetic()).count() >= 4
        && !matches!(n.as_str(), "no subject" | "untitled" | "none" | "re" | "fwd" | "fw" | "tanpa judul")
        && !n.starts_with("indonesia l") && !n.starts_with("indonesia news") && !n.starts_with("indonesia views")
}

/// Choose the title. `subject_title` is the subject without list tags and outlet codes;
/// `topic_reply` marks an "(r)" message, whose subject is the conference topic;
/// `inner` is the title parsed from a Subject: line inside the message, when there is one.
pub fn derive(subject_title: &str, topic_reply: bool, inner: Option<&str>, lead: &[String], text: &str) -> Derived {
    let base = subject_title.trim().trim_end_matches("...").trim_end_matches('.').trim();
    let nb = norm(base);
    if let Some(inner) = inner.map(|s| s.trim().trim_end_matches("...").trim()).filter(|s| usable(s)) {
        let ni = norm(inner);
        if topic_reply && ni != nb {
            return Derived { title: clean_title(inner), source: "inner_subject" };
        }
        if ni.len() > nb.len() && ni.starts_with(&nb) && !nb.is_empty() {
            return Derived { title: clean_title(inner), source: "inner_subject" };
        }
    }
    if nb.chars().count() >= 5 {
        for i in 0..lead.len().min(14) {
            let l = lead[i].trim();
            if l.is_empty() || l.starts_with('>') {
                continue;
            }
            let nl = norm(l);
            let capital = l.chars().find(|c| c.is_alphanumeric()).map(|c| !c.is_lowercase()).unwrap_or(false);
            if capital && nl.len() > nb.len() + 1 && nl.starts_with(&nb) && headline_shape(lead, i, true) {
                return Derived { title: clean_title(l), source: "body_prefix" };
            }
        }
    }
    if !topic_reply {
        return Derived { title: base.to_string(), source: "subject" };
    }
    for l in lead.iter().take(8) {
        if let Some(c) = CONF_TOPIC.captures(l).or_else(|| CONF_TOPIC_GLUED.captures(l)) {
            let t = clean_title(c[1].trim());
            if usable(&t) && t.chars().count() >= 6 && norm(&t) != nb {
                return Derived { title: t, source: "headline" };
            }
        }
    }
    for i in 0..lead.len().min(12) {
        let l = lead[i].trim();
        if l.is_empty() || is_noise(l) || is_byline(l) || is_dateline(l) || is_outlet_line(l) || TOPIC_LINE.is_match(l) {
            continue;
        }
        if headline_shape(lead, i, false) {
            let t = clean_title(l);
            if norm(&t) != nb && usable(&t) && t.split_whitespace().count() >= 2 {
                return Derived { title: t, source: "headline" };
            }
            if t.split_whitespace().count() < 2 {
                // a one-word section label ("Indonesia", "Correction"): the headline may follow
                continue;
            }
        }
        // the first real line is prose: there is no headline
        break;
    }
    if let Some(o) = opening(text) {
        return Derived { title: o, source: "opening" };
    }
    Derived { title: base.to_string(), source: "topic" }
}

/// A title cut by the conference software, judged against a longer one that begins with it.
pub fn extends(short: &str, long: &str) -> bool {
    let (a, b) = (norm(short), norm(long));
    a.chars().count() >= 5 && b.len() > a.len() + 1 && b.starts_with(&a)
}

/// Series numbers at the end of a title ("(2)", "3") are left out when titles are compared.
pub fn without_part(s: &str) -> String {
    TRAILING_NUM.replace(s, "").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lead(s: &str) -> Vec<String> {
        s.lines().map(|l| l.trim().to_string()).collect()
    }
    #[test]
    fn topic_reply_takes_headline() {
        let l = lead("SUHARTO HARUS REALISTIK: DEMOKRATISASI\n\nKarena kita sudah terlanjur menjadi bangsa yang totaliter dibawah pimpinan");
        let d = derive("Law & Liberty", true, None, &l, "SUHARTO HARUS REALISTIK: DEMOKRATISASI\n\nKarena kita sudah terlanjur menjadi bangsa.");
        assert_eq!(d, Derived { title: "Suharto Harus Realistik: Demokratisasi".into(), source: "headline" });
    }
    #[test]
    fn truncated_subject_continues_in_body() {
        let l = lead("WAWANCARA DENGAN CLIFFORD GEERTZ (6)\n\nSelama Orde Baru, \"Saya tidak menyangkal\"");
        let d = derive("Wawancara dengan Clif", true, None, &l, "");
        assert_eq!(d.source, "body_prefix");
        assert_eq!(d.title, "Wawancara dengan Clifford Geertz (6)");
        let l = lead("Kami minta Dubes Irsan menjelaskan\nhal ini");
        assert_eq!(derive("Pertemuan Hati", false, None, &l, "").source, "subject");
    }
    #[test]
    fn byline_and_kicker_are_skipped() {
        let l = lead("[ Return to the Table of Contents ]\n______________\n\nHADIAH TERBESAR\nOleh Bob Jokiman\n\nPernahkah anda mengadopsi seorang bayi");
        let d = derive("Diskusi Agam", true, None, &l, "");
        assert_eq!(d.title, "Hadiah Terbesar");
        let l = lead("New Straits Times [Malaysia]\nMarch 11, 2000\n\nOpinion: Indonesians struggle with change\n\nINDONESIA has been variously described");
        assert_eq!(derive("Religion", true, None, &l, "").title, "Opinion: Indonesians struggle with change");
    }
    #[test]
    fn prose_gets_opening_words() {
        let text = "Reuter, Wamena, Jonathan Thatcher, May 6 - Derek swears he gave up eating people in the 1960s. \"The missionaries told us not to,\" he said.";
        let l = lead("Reuter, Wamena, Jonathan Thatcher, May 6 - Derek swears he gave up eating\npeople in the 1960s.");
        let d = derive("Travel Guides", true, None, &l, text);
        assert_eq!(d.source, "opening");
        assert!(d.title.starts_with("Reuter, Wamena, Jonathan Thatcher, May 6 - Derek swears"), "{}", d.title);
    }
    #[test]
    fn inner_subject_wins_for_replies() {
        let d = derive("Law & Liberty", true, Some("Golkar dan ABRI"), &[], "");
        assert_eq!(d, Derived { title: "Golkar dan ABRI".into(), source: "inner_subject" });
        let d = derive("KMP - Segera Revisi UU Peradilam Mi", false, Some("Segera Revisi UU Peradilan Militer"), &[], "");
        assert_eq!(d.source, "subject");
        let d = derive("Segera Revisi UU Peradilan Mi", false, Some("Segera Revisi UU Peradilan Militer"), &[], "");
        assert_eq!(d.title, "Segera Revisi UU Peradilan Militer");
    }
    #[test]
    fn conference_forwards() {
        let l = lead("Forwarded from GreenNet reg.easttimor conference:\nTopic 353    Timor Event in NYC 5/12\ncscheiner    reg.easttimor 8:22 pm  Apr 12, 1992\n\nPublic meeting");
        assert_eq!(derive("Area Studies: East Timor", true, None, &l, "").title, "Timor Event in NYC 5/12");
        let l = lead("Forwarded from greenleft.news conference:\nTopic 46    Green Left issue 68, August 26Response 34 of 44 peg:greenleft");
        assert_eq!(derive("Periodicals", true, None, &l, "").title, "Green Left issue 68, August 26");
        let l = lead("Reposted from en.announcements conference by apakabar:\n\nTopic 42SierraClub Int'l Alert Network! econeten.announcemen 9:36 pm  Jan  7, 1991\n\nReceived");
        assert_eq!(derive("Writing Letters", true, None, &l, "").title, "SierraClub Int'l Alert Network!");
        let l = lead("Topic 390Nuclear Plant Going AheadResponse 1 of 1 web:kai reg.indonesia11:56 pm May 23, 1991");
        assert_eq!(derive("Nuclear Plant", true, None, &l, "").title, "Nuclear Plant Going Ahead");
        let l = lead("Reposted by apakabar\n\nNote: excerpts only.\n\nIndonesia\n\nGeneral feels the heat");
        assert_eq!(derive("Writing Letters", true, None, &l, "").title, "General feels the heat");
        assert_eq!(opening("According to a story by the U.S. Catholic Conference, the bishops asked for the release of the prisoners. More."), Some("According to a story by the U.S. Catholic Conference, the bishops asked for the release of the prisoners.".to_string()));
        let l = lead("Reposted by apakabar\n\nKAMI MENOLAK\n\nKami menolak");
        assert_eq!(derive("Writing Letters", true, None, &l, "").title, "Kami Menolak");
    }
    #[test]
    fn casing() {
        assert_eq!(smart_case("PERNYATAAN SIKAP KOMITE ANTI KEKERASAN TERHADAP ABRI DAN PKI"), "Pernyataan Sikap Komite Anti Kekerasan Terhadap ABRI dan PKI");
        assert_eq!(smart_case("Tetap seperti ini"), "Tetap seperti ini");
        assert_eq!(smart_case("G30S/PKI: SIAPA DALANGNYA?"), "G30S/PKI: Siapa Dalangnya?");
        assert_eq!(smart_case("BANJIR DI BANDUNG MAKIN MELUAS"), "Banjir di Bandung Makin Meluas");
        assert_eq!(smart_case("HRW/ASIA - EAST TIMOR GUERRILLA ATTACKS"), "HRW/ASIA - East Timor Guerrilla Attacks");
    }
}
