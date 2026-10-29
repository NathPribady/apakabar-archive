//! Turns a raw hypermail body into readable text.
//!
//! The raw text is always kept as recovered; this produces a separate clean
//! version. It does four things:
//!
//! - decodes quoted-printable remnants and repairs lost characters;
//! - moves nested mail headers ("Forwarded message: From ... Date ...") out of the body;
//! - rejoins hard-wrapped lines into paragraphs;
//! - keeps tables, poems and quoted replies (">") line by line.
//!
//! Output format: paragraphs separated by "\n\n". Inside a paragraph a "\n"
//! is a kept line break. A paragraph starting with "> " is a quotation.

use once_cell::sync::Lazy;
use regex::Regex;

#[derive(Debug, Default, Clone)]
pub struct Cleaned {
    pub text: String,
    pub headers: String,
    pub orig_from: String,
    pub orig_date: String,
    pub orig_subject: String,
    pub x_url: String,
    pub repaired_chars: usize,
    pub qp_decoded: bool,
    /// first non-empty body lines, as written (for finding headlines)
    pub lead: Vec<String>,
    /// HTML entities or tags left by the capture that were decoded or removed
    pub html_fixed: usize,
}

static HEADER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(from|date|to|cc|bcc|subject|sender|reply-to|message-id|organi[sz]ation|precedence|errors-to|return-path|received|mime-version|content-[a-z-]+|x-[a-z0-9-]+|in-reply-to|references|status|lines|apparently-to|comments|priority|importance|delivered-to|list-[a-z-]+|resent-[a-z-]+)\s*:").unwrap()
});
static MBOX_FROM: Lazy<Regex> = Lazy::new(|| Regex::new(r"^From \S+\s+\w{3}\s+\w{3}\s+\d").unwrap());
static FWD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*(-{2,}\s*)?(forwarded message|original message|begin forwarded message)\b.*$").unwrap()
});
/// Conference headers of the IGC, PeaceNet and GreenNet systems:
/// /* Written 3:19 PM Jun 7, 1995 by gn:cdpm in igc:reg.easttimor */ and /* ---------- "Title" ---------- */
static CONF_HDR: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*/\*\s*(Written\b.*|-{4,}.*)\*/\s*$").unwrap());
static XURL: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^x-[a-z-]*url\s*:").unwrap());
static LIST_BANNER: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(INDONESIA-[A-Z]+|IRI-L|APAKABAR|INDONESIA-L|_{10,}|-{10,}|={10,})$").unwrap());
static QP_HEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"=([0-9A-F]{2})").unwrap());
static ENTITY: Lazy<Regex> = Lazy::new(|| Regex::new(r"&(?:#(\d{1,7})|#[xX]([0-9a-fA-F]{1,6})|([a-zA-Z]{2,8}));?").unwrap());
static TAG: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)</?(?:a|b|i|u|s|p|br|hr|font|em|strong|pre|div|span|blockquote|center|table|tbody|thead|tr|td|th|ul|ol|li|h[1-6]|img|html|head|body|title|meta|tt|small|big|sup|sub|dl|dt|dd|address|cite|code|base|link|strike|nobr|wbr)(?:\s[^<>]{0,400})?\s*/?>").unwrap()
});

fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "quot" => "\"", "amp" => "&", "lt" => "<", "gt" => ">", "nbsp" => " ", "apos" => "'", "copy" => "\u{a9}", "reg" => "\u{ae}",
        "eacute" => "\u{e9}", "egrave" => "\u{e8}", "aacute" => "\u{e1}", "agrave" => "\u{e0}", "ccedil" => "\u{e7}", "atilde" => "\u{e3}",
        "otilde" => "\u{f5}", "oacute" => "\u{f3}", "uacute" => "\u{fa}", "iacute" => "\u{ed}", "ntilde" => "\u{f1}", "uuml" => "\u{fc}",
        "ouml" => "\u{f6}", "auml" => "\u{e4}", "ecirc" => "\u{ea}", "ocirc" => "\u{f4}", "acirc" => "\u{e2}", "szlig" => "\u{df}",
        "Eacute" => "\u{c9}", "Aacute" => "\u{c1}", "laquo" => "\u{ab}", "raquo" => "\u{bb}", "ndash" => "\u{2013}", "mdash" => "\u{2014}",
        "lsquo" => "\u{2018}", "rsquo" => "\u{2019}", "ldquo" => "\u{201c}", "rdquo" => "\u{201d}", "hellip" => "\u{2026}", "bull" => "\u{2022}",
        "middot" => "\u{b7}", "deg" => "\u{b0}", "pound" => "\u{a3}", "euro" => "\u{20ac}", "yen" => "\u{a5}", "sect" => "\u{a7}", "para" => "\u{b6}",
        "times" => "\u{d7}", "divide" => "\u{f7}", "frac12" => "\u{bd}", "frac14" => "\u{bc}", "shy" => "",
        _ => return None,
    })
}

fn code_point(n: u32) -> String {
    match n {
        9 | 10 | 13 => char::from_u32(n).map(|c| c.to_string()).unwrap_or_default(),
        0..=31 | 127 => String::new(),
        160 => " ".into(),
        145 | 146 => "\u{2019}".into(),
        147 | 148 => "\"".into(),
        150 | 151 => "\u{2013}".into(),
        133 => "\u{2026}".into(),
        128..=159 => String::new(),
        _ => char::from_u32(n).map(|c| c.to_string()).unwrap_or_default(),
    }
}

/// Removes HTML tags left by the capture and decodes character entities (&quot;, &#127, &nbsp;).
/// Tags are matched by name, so e-mail addresses in angle brackets (<x@y.org>) are kept.
pub fn fix_html(s: &str) -> (String, usize) {
    let mut n = 0;
    let mut out = if s.contains('<') && TAG.is_match(s) {
        TAG.replace_all(s, |c: &regex::Captures| {
            n += 1;
            let t = c[0].to_ascii_lowercase();
            if t.starts_with("<br") || t.starts_with("<p") || t.starts_with("</p") || t.starts_with("<hr") || t.starts_with("<div") || t.starts_with("</div") || t.starts_with("<tr") || t.starts_with("<li") { "\n".to_string() } else { String::new() }
        })
        .into_owned()
    } else {
        s.to_string()
    };
    if out.contains('&') {
        out = ENTITY
            .replace_all(&out, |c: &regex::Captures| {
                if let Some(d) = c.get(1) {
                    n += 1;
                    return code_point(d.as_str().parse().unwrap_or(0));
                }
                if let Some(h) = c.get(2) {
                    n += 1;
                    return code_point(u32::from_str_radix(h.as_str(), 16).unwrap_or(0));
                }
                let name = c.get(3).map(|m| m.as_str()).unwrap_or("");
                match named_entity(name) {
                    // a bare "&amp" without ";" in running text is kept as written
                    Some(v) if c[0].ends_with(';') || matches!(name, "quot" | "nbsp" | "amp" | "lt" | "gt") => {
                        n += 1;
                        v.to_string()
                    }
                    _ => c[0].to_string(),
                }
            })
            .into_owned();
    }
    (out, n)
}

/// Quoted-printable spaces ("=20") left in messages that are otherwise not encoded.
fn fix_qp_spaces(s: &str) -> String {
    if !s.contains("=20") && !s.contains("=3D") {
        return s.to_string();
    }
    let mut t = s.replace("=20\n", "\n").replace("=20 ", "  ").replace("=3D", "=");
    while t.ends_with("=20") {
        t.truncate(t.len() - 3);
    }
    t
}

fn decode_qp(s: &str) -> String {
    let s = s.replace("=\n", "");
    QP_HEX
        .replace_all(&s, |c: &regex::Captures| {
            let b = u8::from_str_radix(&c[1], 16).unwrap_or(b'?');
            match b {
                0x20 => " ".to_string(),
                0x3D => "=".to_string(),
                0x09 => "\t".to_string(),
                0x0A => "\n".to_string(),
                0x91 | 0x92 => "\u{2019}".to_string(),
                0x93 | 0x94 => "\"".to_string(),
                0x96 | 0x97 => "\u{2013}".to_string(),
                b if b < 0x80 => (b as char).to_string(),
                _ => (b as char).to_string(), // latin-1 guess
            }
        })
        .into_owned()
}

/// U+FFFD replacement characters come from Windows-1252 quotes lost in capture:
/// between two letters it was an apostrophe (Jum'at, Da'wah); elsewhere a quote mark.
fn repair_fffd(s: &str) -> (String, usize) {
    if !s.contains('\u{FFFD}') {
        return (s.to_string(), 0);
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut n = 0;
    for i in 0..chars.len() {
        let c = chars[i];
        if c == '\u{FFFD}' {
            n += 1;
            let prev = if i > 0 { chars[i - 1] } else { ' ' };
            let next = if i + 1 < chars.len() { chars[i + 1] } else { ' ' };
            if prev.is_alphanumeric() && next.is_alphanumeric() {
                out.push('\u{2019}');
            } else if next == '\u{FFFD}' || prev == '\u{FFFD}' {
                // runs of several replacement chars: drop the extras
                if prev != '\u{FFFD}' {
                    out.push('"');
                }
            } else {
                out.push('"');
            }
        } else {
            out.push(c);
        }
    }
    (out, n)
}

fn is_header_line(l: &str) -> bool {
    let l = l.trim_start_matches(|c| c == '>' || c == ' ');
    HEADER.is_match(l) || MBOX_FROM.is_match(l)
}

fn header_value(l: &str) -> (String, String) {
    match l.find(':') {
        Some(i) => (l[..i].trim().to_ascii_lowercase(), l[i + 1..].trim().to_string()),
        None => (String::new(), l.trim().to_string()),
    }
}

pub fn clean(raw: &str) -> Cleaned {
    let mut c = Cleaned::default();
    let mut s = raw.replace("\r\n", "\n").replace('\r', "\n").replace('\t', "    ").replace('\u{a0}', " ");
    let qp_signals = s.matches("=20").count() + s.matches("=\n").count() + s.matches("=3D").count();
    if qp_signals >= 3 {
        s = decode_qp(&s);
        c.qp_decoded = true;
    } else {
        s = fix_qp_spaces(&s);
    }
    let (s, nh) = fix_html(&s);
    c.html_fixed = nh;
    let (s, n) = repair_fffd(&s);
    c.repaired_chars = n;

    let lines: Vec<&str> = s.lines().collect();
    let mut i = 0;
    // hypermail repeats the date as the first line
    if lines.first().map(|l| l.starts_with("Date: ")).unwrap_or(false) {
        i = 1;
    }
    let mut body_lines: Vec<&str> = Vec::with_capacity(lines.len());
    let mut headers: Vec<String> = Vec::new();
    while i < lines.len() {
        let l = lines[i];
        let lt = l.trim();
        if XURL.is_match(lt) {
            let (_, v) = header_value(lt);
            c.x_url = v;
            headers.push(lt.to_string());
            i += 1;
            continue;
        }
        if body_lines.iter().all(|b| b.trim().is_empty()) && LIST_BANNER.is_match(lt) {
            headers.push(lt.to_string());
            i += 1;
            continue;
        }
        if CONF_HDR.is_match(l) {
            let t = l.trim();
            // the quoted title in /* ---------- "Title" ---------- */ is the posting's own subject
            if let (Some(a), Some(b)) = (t.find('"'), t.rfind('"')) {
                if b > a + 1 && c.orig_subject.is_empty() {
                    c.orig_subject = t[a + 1..b].to_string();
                }
            }
            headers.push(t.to_string());
            i += 1;
            continue;
        }
        if FWD.is_match(l) {
            headers.push(l.trim().to_string());
            i += 1;
            continue;
        }
        if is_header_line(l) && (!l.trim_start().starts_with('>') || body_lines.iter().all(|b| b.trim().is_empty())) {
            // measure the block: header lines + continuation lines, until blank or prose
            let mut j = i;
            let mut hdr_count = 0;
            while j < lines.len() {
                let lj = lines[j];
                if is_header_line(lj) {
                    hdr_count += 1;
                    j += 1;
                } else if j > i && hdr_count > 0 && (lj.starts_with(' ') || lj.starts_with('\t')) && !lj.trim().is_empty() && looks_like_continuation(lj) {
                    j += 1;
                } else {
                    break;
                }
            }
            // a lone "From: X" or "Date:" at the top of a message is also header material;
            // a single "To:" in prose is not. Accept blocks of >= 2 lines, or single
            // lines before the first prose line (X-URL, From:, Date: on their own).
            let single_ok = hdr_count == 1 && body_lines.iter().all(|b| b.trim().is_empty()) && {
                let (k, _) = header_value(l);
                matches!(k.as_str(), "from" | "date" | "subject" | "x-url" | "to" | "sender") || MBOX_FROM.is_match(l)
            };
            if hdr_count >= 2 || single_ok {
                for lh in &lines[i..j] {
                    let (k, v) = header_value(lh);
                    match k.as_str() {
                        "from" => c.orig_from = v.clone(),
                        "date" => c.orig_date = v.clone(),
                        "subject" => c.orig_subject = v.clone(),
                        "x-url" => c.x_url = v.clone(),
                        _ => {}
                    }
                    headers.push(lh.trim_end().to_string());
                }
                i = j;
                continue;
            }
        }
        body_lines.push(l);
        i += 1;
    }
    c.headers = headers.join("\n");
    // first lines of the text as written, one "" for each run of blank lines
    for l in &body_lines {
        let t = l.trim();
        if t.is_empty() {
            if c.lead.last().map(|x| !x.is_empty()).unwrap_or(false) {
                c.lead.push(String::new());
            }
        } else {
            c.lead.push(t.to_string());
        }
        if c.lead.len() >= 20 {
            break;
        }
    }
    c.text = paragraphs(&body_lines);
    c
}

fn looks_like_continuation(l: &str) -> bool {
    let t = l.trim();
    // address lists, folded subjects, Received: hops
    t.contains('@') || t.contains(';') || t.contains("ESMTP") || t.starts_with('<') || t.starts_with('(')
        || ["id ", "by ", "for ", "from ", "with ", "via "].iter().any(|p| t.starts_with(p))
        || t.split_whitespace().count() <= 6
}

fn is_pre_line(l: &str) -> bool {
    let t = l.trim_start();
    if t.is_empty() {
        return false;
    }
    // internal runs of 3+ spaces = table columns
    let inner = t.trim_end();
    inner.contains("   ") || inner.contains(" | ") || inner.starts_with('|') || inner.chars().filter(|c| *c == '.').count() > 8 && inner.contains("....")
}

fn ends_sentence(l: &str) -> bool {
    let t = l.trim_end();
    t.ends_with('.') || t.ends_with('?') || t.ends_with('!') || t.ends_with(':') || t.ends_with('"') || t.ends_with('\u{201D}') || t.ends_with(')')
}

/// Build paragraphs from body lines.
fn paragraphs(lines: &[&str]) -> String {
    // group into blocks separated by blank lines
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for l in lines {
        if l.trim().is_empty() {
            if !cur.is_empty() {
                blocks.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(l.trim_end());
        }
    }
    if !cur.is_empty() {
        blocks.push(cur);
    }
    let mut out: Vec<String> = Vec::new();
    for b in blocks {
        // quotation blocks
        if b.iter().all(|l| l.trim_start().starts_with('>')) {
            let q: Vec<String> = b.iter().map(|l| l.trim_start().trim_start_matches('>').trim_start_matches('>').trim().to_string()).collect();
            out.push(format!("> {}", join_prose(&q.iter().map(|s| s.as_str()).collect::<Vec<_>>()).replace("\n\n", "\n")));
            continue;
        }
        let pre = b.iter().filter(|l| is_pre_line(l)).count();
        let short = b.iter().filter(|l| l.trim().chars().count() < 38).count();
        let longest = b.iter().map(|l| l.trim().chars().count()).max().unwrap_or(0);
        let poem_like = b.len() >= 4 && short * 10 >= b.len() * 8 && longest < 48;
        if pre * 3 >= b.len() || poem_like || (b.len() >= 3 && b.iter().all(|l| l.trim_start().starts_with(['-', '*', '+', 'o']) || l.trim_start().chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))) {
            out.push(b.iter().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n"));
            continue;
        }
        let joined = join_prose(&b);
        for p in joined.split("\n\n") {
            if !p.trim().is_empty() {
                out.push(p.trim().to_string());
            }
        }
    }
    out.join("\n\n")
}

/// Join hard-wrapped lines; an indented line after a finished sentence starts a new paragraph.
fn join_prose(lines: &[&str]) -> String {
    let mut s = String::new();
    let mut prev: Option<&str> = None;
    for l in lines {
        let indented = l.starts_with("   ") || l.starts_with('\t');
        let t = l.trim();
        match prev {
            None => s.push_str(t),
            Some(p) => {
                let p_short = p.trim().chars().count() < 45;
                if (indented && ends_sentence(p)) || (p_short && ends_sentence(p) && t.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)) {
                    s.push_str("\n\n");
                    s.push_str(t);
                } else if p.trim_end().ends_with('-') && !p.trim_end().ends_with(" -") && t.chars().next().map(|c| c.is_lowercase()).unwrap_or(false) {
                    // hyphenated word split across lines ("kekiri-\nkirian" keeps its hyphen)
                    s.push_str(t);
                } else {
                    s.push(' ');
                    s.push_str(t);
                }
            }
        }
        prev = Some(l);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn headers_and_paragraphs() {
        let raw = "Date: Fri May 01 1998 - 13:44:00 EDT\n\nForwarded message:\nFrom apakabar@clark.net Fri May  1 16:43:29 1998\nDate: Fri, 1 May 1998 14:41:42 -0600 (MDT) \nTo: indonesia-l@indopubs.com \nFrom: apakabar@clark.net \nSubject: [INDONESIA-L] Gerakan Mahasiswa \n \n\nDate: Thu, 30 Apr 1998 18:19:15 +0700  \nFrom: Shiddiq <x@y>  \nSubject: Gerakan Mahasiswa, Mau Kemana?  \n \nGERAKAN MAHASISWA,   \nMAU KEMANA?  \n  \n        Kini Mahasiswa pun demo di luar kampus. Jum\u{FFFD}at (10/4/98), usai shalat  \nJum\u{FFFD}at, sekitar 20.000 massa.  \n        Aksi berbendera KAMMI, organisasi  \ninformal baru.\n";
        let c = clean(raw);
        assert_eq!(c.orig_from, "Shiddiq <x@y>");
        assert!(c.orig_subject.starts_with("Gerakan Mahasiswa, Mau"));
        assert!(c.text.contains("Jum\u{2019}at (10/4/98), usai shalat Jum\u{2019}at"), "{}", c.text);
        assert!(c.text.contains("massa.\n\nAksi berbendera KAMMI, organisasi informal baru."), "{}", c.text);
        assert!(!c.text.contains("Message-Id"));
    }
    #[test]
    fn html_leftovers() {
        let (t, n) = fix_html("Tim&#127; Relawan &quot;menolak&quot; <b>tuduhan</b> &amp; hoax<br>Baris dua. Tulis ke <tapol@gn.apc.org>.");
        assert_eq!(t, "Tim Relawan \"menolak\" tuduhan & hoax\nBaris dua. Tulis ke <tapol@gn.apc.org>.");
        assert!(n >= 5);
        let (t, _) = fix_html("AT&T and R&D stay; &nbsp;space");
        assert_eq!(t, "AT&T and R&D stay;  space");
        assert_eq!(fix_qp_spaces("satu=20\ndua =20 tiga=20"), "satu\ndua   tiga");
    }
    #[test]
    fn conference_headers() {
        let c = clean("/* Written 3:19 PM Jun 7, 1995 by gn:cdpm in igc:reg.easttimor */\n/* ---------- \"XG statement at IPCET\" ---------- */\nStatement by the leader of the Resistance.");
        assert_eq!(c.orig_subject, "XG statement at IPCET");
        assert!(c.text.starts_with("Statement by the leader"), "{}", c.text);
    }
    #[test]
    fn qp() {
        let c = clean("Hello=20world=3D=\nok and =20 more =20");
        assert!(c.text.starts_with("Hello world=ok"), "{}", c.text);
    }
}
