//! Press sources, subject parsing, message types.
//!
//! Most apakabar messages repost an article. MacDougall put the outlet's code
//! at the front of the subject ("KMP - ...", "JKTP - ...", "IN: AFP - ..."),
//! so the code usually tells us where the text came from.

use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;

pub struct Source {
    pub code: &'static str,
    pub name: &'static str,
    pub category: &'static str,
}

/// Categories: id_daily, id_magazine, id_online, id_wire, alternative, wire, foreign, ngo,
/// list_rubric, contributor.
pub static SOURCES: &[Source] = &[
    // Indonesian dailies
    Source { code: "KMP", name: "Kompas", category: "id_daily" },
    Source { code: "RPK", name: "Republika", category: "id_daily" },
    Source { code: "PMB", name: "Suara Pembaruan", category: "id_daily" },
    Source { code: "SM", name: "Suara Merdeka", category: "id_daily" },
    Source { code: "MI", name: "Media Indonesia", category: "id_daily" },
    Source { code: "MEDIA", name: "Media Indonesia", category: "id_daily" },
    Source { code: "JWP", name: "Jawa Pos", category: "id_daily" },
    Source { code: "PR", name: "Pikiran Rakyat", category: "id_daily" },
    Source { code: "BI", name: "Bisnis Indonesia", category: "id_daily" },
    Source { code: "SBYP", name: "Surabaya Post", category: "id_daily" },
    Source { code: "SERAMBI", name: "Serambi Indonesia", category: "id_daily" },
    Source { code: "WSPD", name: "Waspada", category: "id_daily" },
    Source { code: "BERNAS", name: "Bernas", category: "id_daily" },
    Source { code: "BALIPOST", name: "Bali Post", category: "id_daily" },
    Source { code: "BP", name: "Bali Post (probable)", category: "id_daily" },
    Source { code: "DUTA", name: "Duta Masyarakat", category: "id_daily" },
    Source { code: "KONTAN", name: "Kontan", category: "id_magazine" },
    Source { code: "SK", name: "Suara Karya", category: "id_daily" },
    Source { code: "MERDEKA", name: "Merdeka", category: "id_daily" },
    Source { code: "POSKOTA", name: "Pos Kota", category: "id_daily" },
    Source { code: "SURYA", name: "Surya", category: "id_daily" },
    Source { code: "PELITA", name: "Pelita", category: "id_daily" },
    Source { code: "NERACA", name: "Neraca", category: "id_daily" },
    Source { code: "JKTP", name: "The Jakarta Post", category: "id_daily" },
    Source { code: "IO", name: "Indonesian Observer", category: "id_daily" },
    Source { code: "KCM", name: "Kompas Cyber Media", category: "id_online" },
    // news magazines and their online successors
    Source { code: "TEMPO", name: "Tempo", category: "id_magazine" },
    Source { code: "TI", name: "Tempo Interaktif", category: "id_online" },
    Source { code: "GATRA", name: "Gatra", category: "id_magazine" },
    Source { code: "FORUM", name: "Forum Keadilan", category: "id_magazine" },
    Source { code: "UMMAT", name: "Ummat", category: "id_magazine" },
    Source { code: "PANJI", name: "Panji Masyarakat", category: "id_magazine" },
    Source { code: "GAMMA", name: "Gamma", category: "id_magazine" },
    Source { code: "TAJUK", name: "Tajuk", category: "id_magazine" },
    Source { code: "D&R", name: "D&R (Detektif & Romantika)", category: "id_magazine" },
    Source { code: "ADIL", name: "Adil", category: "id_magazine" },
    Source { code: "EDITOR", name: "Editor", category: "id_magazine" },
    Source { code: "SINAR", name: "Sinar", category: "id_magazine" },
    Source { code: "DETIK", name: "DeTIK / detik.com", category: "id_online" },
    Source { code: "ASTAGA", name: "astaga.com", category: "id_online" },
    Source { code: "ANTARA", name: "Antara (state news agency)", category: "id_wire" },
    // alternative and underground
    Source { code: "SIAR", name: "SiaR (underground news service)", category: "alternative" },
    Source { code: "KDP", name: "Kabar dari PIJAR", category: "alternative" },
    Source { code: "PIJAR", name: "PIJAR", category: "alternative" },
    Source { code: "XPOS", name: "Xpos", category: "alternative" },
    Source { code: "PRD", name: "Partai Rakyat Demokratik", category: "alternative" },
    Source { code: "AJI", name: "Aliansi Jurnalis Independen", category: "alternative" },
    Source { code: "INDEPENDEN", name: "Suara Independen", category: "alternative" },
    Source { code: "SIRA", name: "SIRA (Aceh referendum movement)", category: "alternative" },
    // wire services
    Source { code: "AFP", name: "Agence France-Presse", category: "wire" },
    Source { code: "RTR", name: "Reuters", category: "wire" },
    Source { code: "REUTERS", name: "Reuters", category: "wire" },
    Source { code: "AP", name: "Associated Press", category: "wire" },
    Source { code: "AFX", name: "AFX News", category: "wire" },
    Source { code: "DJ", name: "Dow Jones Newswires", category: "wire" },
    Source { code: "KYODO", name: "Kyodo News", category: "wire" },
    Source { code: "IPS", name: "Inter Press Service", category: "wire" },
    Source { code: "LUSA", name: "Lusa (Portuguese news agency)", category: "wire" },
    Source { code: "UPI", name: "United Press International", category: "wire" },
    Source { code: "BERNAMA", name: "Bernama", category: "wire" },
    // foreign press and broadcasters
    Source { code: "SMH", name: "Sydney Morning Herald", category: "foreign" },
    Source { code: "AGE", name: "The Age", category: "foreign" },
    Source { code: "ST", name: "The Straits Times", category: "foreign" },
    Source { code: "BT", name: "Business Times (Singapore)", category: "foreign" },
    Source { code: "SCMP", name: "South China Morning Post", category: "foreign" },
    Source { code: "WASHPOST", name: "The Washington Post", category: "foreign" },
    Source { code: "NYT", name: "The New York Times", category: "foreign" },
    Source { code: "FEER", name: "Far Eastern Economic Review", category: "foreign" },
    Source { code: "ASIAWEEK", name: "Asiaweek", category: "foreign" },
    Source { code: "AWSJ", name: "Asian Wall Street Journal", category: "foreign" },
    Source { code: "WSJ", name: "The Wall Street Journal", category: "foreign" },
    Source { code: "ABC", name: "Australian Broadcasting Corporation", category: "foreign" },
    Source { code: "VOA", name: "Voice of America", category: "foreign" },
    Source { code: "DW", name: "Deutsche Welle", category: "foreign" },
    Source { code: "BBC", name: "BBC", category: "foreign" },
    Source { code: "MSIA", name: "Malaysian press", category: "foreign" },
    Source { code: "NST", name: "New Straits Times", category: "foreign" },
    Source { code: "TIME", name: "Time", category: "foreign" },
    Source { code: "NEWSWEEK", name: "Newsweek", category: "foreign" },
    Source { code: "IHT", name: "International Herald Tribune", category: "foreign" },
    Source { code: "GUARDIAN", name: "The Guardian", category: "foreign" },
    Source { code: "ECONOMIST", name: "The Economist", category: "foreign" },
    Source { code: "CSM", name: "Christian Science Monitor", category: "foreign" },
    Source { code: "LAT", name: "Los Angeles Times", category: "foreign" },
    Source { code: "AUSTRALIAN", name: "The Australian", category: "foreign" },
    Source { code: "GLW", name: "Green Left Weekly", category: "foreign" },
    // NGOs
    Source { code: "AI", name: "Amnesty International", category: "ngo" },
    Source { code: "HRW", name: "Human Rights Watch", category: "ngo" },
    Source { code: "TAPOL", name: "TAPOL", category: "ngo" },
    Source { code: "ETAN", name: "East Timor Action Network", category: "ngo" },
    Source { code: "ELSAM", name: "ELSAM", category: "ngo" },
    Source { code: "YLBHI", name: "YLBHI", category: "ngo" },
    Source { code: "KONTRAS", name: "KontraS", category: "ngo" },
    Source { code: "WALHI", name: "WALHI", category: "ngo" },
    Source { code: "INFID", name: "INFID", category: "ngo" },
    Source { code: "ISAI", name: "ISAI", category: "ngo" },
    Source { code: "KIPP", name: "KIPP (election monitors)", category: "ngo" },
    // apakabar's own rubrics
    Source { code: "ISSUES", name: "Issues (apakabar discussion rubric)", category: "list_rubric" },
    Source { code: "RUBRIK", name: "Rubrik (apakabar rubric)", category: "list_rubric" },
    Source { code: "RESOURCES", name: "Resources (apakabar rubric)", category: "list_rubric" },
    Source { code: "KOLOM", name: "Kolom (columns)", category: "list_rubric" },
    Source { code: "TIMOR", name: "Timor (apakabar rubric)", category: "list_rubric" },
    Source { code: "ACEH", name: "Aceh (apakabar rubric)", category: "list_rubric" },
    // regular contributors with their own subject codes
    Source { code: "JOSHUA", name: "Joshua (contributor)", category: "contributor" },
    Source { code: "STEFANUS", name: "Stefanus (contributor)", category: "contributor" },
    Source { code: "JUSFIQ", name: "Jusfiq Hadjar (contributor)", category: "contributor" },
    Source { code: "FARIED", name: "Faried (contributor)", category: "contributor" },
    Source { code: "SUPANGKAT", name: "Supangkat (contributor)", category: "contributor" },
    Source { code: "PROLETAR", name: "Proletar (contributor)", category: "contributor" },
    Source { code: "SOBRON", name: "Sobron Aidit (contributor)", category: "contributor" },
    Source { code: "LENG", name: "Leng (contributor)", category: "contributor" },
    // more outlets found in the subjects
    Source { code: "KOMPAS", name: "Kompas", category: "id_daily" },
    Source { code: "JAWAPOS", name: "Jawa Pos", category: "id_daily" },
    Source { code: "WASPADA", name: "Waspada", category: "id_daily" },
    Source { code: "RIAUPOS", name: "Riau Pos", category: "id_daily" },
    Source { code: "KR", name: "Kedaulatan Rakyat", category: "id_daily" },
    Source { code: "SH", name: "Sinar Harapan", category: "id_daily" },
    Source { code: "RM", name: "Rakyat Merdeka", category: "id_daily" },
    Source { code: "ANALISA", name: "Analisa (Medan)", category: "id_daily" },
    Source { code: "POSKUP", name: "Pos Kupang", category: "id_daily" },
    Source { code: "TIRAS", name: "Tiras", category: "id_magazine" },
    Source { code: "PANJIMAS", name: "Panji Masyarakat", category: "id_magazine" },
    Source { code: "SWA", name: "SWA", category: "id_magazine" },
    Source { code: "WE", name: "Warta Ekonomi", category: "id_magazine" },
    Source { code: "KAPITAL", name: "Kapital", category: "id_magazine" },
    Source { code: "DETIKCOM", name: "DeTIK / detik.com", category: "id_online" },
    Source { code: "SATUNET", name: "satunet.com", category: "id_online" },
    Source { code: "PEMBEBASAN", name: "Pembebasan (PRD)", category: "alternative" },
    Source { code: "RTRS", name: "Reuters", category: "wire" },
    Source { code: "AFR", name: "Australian Financial Review", category: "foreign" },
    Source { code: "AT", name: "Asia Times", category: "foreign" },
    Source { code: "ASIATIMES", name: "Asia Times", category: "foreign" },
    Source { code: "WP", name: "The Washington Post", category: "foreign" },
    Source { code: "WASHTIMES", name: "The Washington Times", category: "foreign" },
    Source { code: "NATION", name: "The Nation (Bangkok)", category: "foreign" },
    Source { code: "STAR", name: "The Star (Malaysia)", category: "foreign" },
    Source { code: "RNW", name: "Radio Nederland", category: "foreign" },
    Source { code: "GL", name: "Green Left Weekly", category: "foreign" },
    Source { code: "THAI", name: "Thai press (rubric)", category: "foreign" },
    Source { code: "SING", name: "Singapore press (rubric)", category: "foreign" },
    Source { code: "PHIL", name: "Philippine press (rubric)", category: "foreign" },
    Source { code: "ASIET", name: "ASIET (Action in Solidarity with Indonesia and East Timor)", category: "ngo" },
    Source { code: "SOLIDAMOR", name: "Solidamor", category: "ngo" },
    Source { code: "WB", name: "World Bank", category: "intl_org" },
    Source { code: "INDOEMBDC", name: "Indonesian Embassy, Washington", category: "government" },
    Source { code: "USEMBJKT", name: "US Embassy, Jakarta", category: "government" },
    Source { code: "POLRI", name: "Indonesian National Police", category: "government" },
    Source { code: "BPMSPRES", name: "Presidential press bureau", category: "government" },
    Source { code: "IRJA", name: "Irian Jaya (apakabar rubric)", category: "list_rubric" },
    Source { code: "MALUKU", name: "Maluku (apakabar rubric)", category: "list_rubric" },
    Source { code: "SULSEL", name: "South Sulawesi (apakabar rubric)", category: "list_rubric" },
    Source { code: "JATIM", name: "East Java (apakabar rubric)", category: "list_rubric" },
    Source { code: "BIO", name: "Biodata (apakabar rubric)", category: "list_rubric" },
    Source { code: "DISS", name: "Dissertations (apakabar rubric)", category: "list_rubric" },
];

pub static BY_CODE: Lazy<HashMap<&'static str, &'static Source>> = Lazy::new(|| SOURCES.iter().map(|s| (s.code, s)).collect());

pub fn category_label(cat: &str) -> &'static str {
    match cat {
        "id_daily" => "Indonesian dailies",
        "id_magazine" => "Indonesian news magazines",
        "id_online" => "Indonesian online news",
        "id_wire" => "Antara (state agency)",
        "alternative" => "Alternative and underground",
        "wire" => "International wire services",
        "foreign" => "Foreign press and broadcasters",
        "ngo" => "Human rights and other NGOs",
        "list_rubric" => "Apakabar rubrics",
        "contributor" => "Regular contributors",
        "uncoded" => "No source code",
        "other_code" => "Other codes (mostly individual contributors)",
        "government" => "Government offices",
        "intl_org" => "International organisations",
        _ => "Other codes",
    }
}

static DOMAINS: &[(&str, &str)] = &[
    ("kompas.com", "KMP"), ("thejakartapost.com", "JKTP"), ("detik.com", "DETIK"), ("tempo.co.id", "TI"),
    ("tempointeraktif", "TI"), ("republika.co.id", "RPK"), ("suarapembaruan", "PMB"), ("mediaindo", "MI"),
    ("gatra.com", "GATRA"), ("suaramerdeka", "SM"), ("jawapos", "JWP"), ("pikiran-rakyat", "PR"), ("bisnis.com", "BI"),
    ("waspada", "WSPD"), ("antara.co.id", "ANTARA"), ("smh.com.au", "SMH"), ("straitstimes", "ST"), ("scmp.com", "SCMP"),
    ("washingtonpost", "WASHPOST"), ("nytimes.com", "NYT"), ("feer.com", "FEER"), ("asiaweek", "ASIAWEEK"),
    ("abc.net.au", "ABC"), ("bbc.co.uk", "BBC"), ("theage.com.au", "AGE"), ("serambinews", "SERAMBI"),
    ("balipost", "BALIPOST"), ("bernas", "BERNAS"), ("observer.co.id", "IO"), ("kontan", "KONTAN"),
];

pub fn code_from_url(url: &str) -> Option<&'static str> {
    let u = url.to_lowercase();
    DOMAINS.iter().find(|(d, _)| u.contains(d)).map(|(_, c)| *c)
}

static TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*\[[^\]]{1,40}\]\s*").unwrap());
static LEAD: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^\s*(in(?:/[a-z]+)?|fwd?|fw|re|aw)\s*:\s*").unwrap());
static SIAR: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^\s*(siar|xpos)\s*-{1,3}>\s*(xpos\s*:\s*)?|^\s*siar\s*-{2,3}\s*").unwrap());
static CODE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*([A-Za-z][A-Za-z&]{1,11})\s*(?:-|:|/)\s+").unwrap());
static TRAIL: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*\((r|R)\)\s*$").unwrap());
static PARTS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*[\(\[]?\b\d{1,2}\s*/\s*\d{1,2}\b[\)\]]?\s*$").unwrap());
static LISTTAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[([A-Za-z][A-Za-z0-9-]{2,30})\]").unwrap());

#[derive(Debug, Default, Clone)]
pub struct Subject {
    pub title: String,
    pub code: Option<String>,
    pub is_reply: bool,
    /// "(r)": a reply filed under a conference topic; the subject is the topic's name
    pub topic_reply: bool,
    pub list_tag: Option<String>,
    pub thread_key: String,
}

/// Parse a subject such as "[INDONESIA-L] IN: AFP - Golkar Divided (r)".
pub fn parse_subject(raw: &str) -> Subject {
    let mut s = raw.trim().to_string();
    let mut out = Subject::default();
    if let Some(m) = LISTTAG.captures(&s) {
        out.list_tag = Some(m[1].to_lowercase());
    }
    // hypermail list archives put "indonesia-p Archives: " in front
    if let Some(i) = s.find(" Archives: ") {
        if i < 30 {
            s = s[i + 11..].to_string();
        }
    }
    if let Some(i) = s.find(": [") {
        if i < 20 && s[..i].starts_with("\"") {
            s = s[i + 2..].to_string();
        }
    }
    s = s.trim_start_matches('"').to_string();
    if TRAIL.is_match(&s) {
        out.topic_reply = true;
        s = TRAIL.replace(&s, "").into_owned();
    }
    for _ in 0..5 {
        let before = s.clone();
        s = TAG.replace(&s, "").into_owned();
        if let Some(m) = LEAD.captures(&s) {
            if m[1].eq_ignore_ascii_case("re") || m[1].eq_ignore_ascii_case("aw") {
                out.is_reply = true;
            }
            s = s[m.get(0).unwrap().end()..].to_string();
        }
        if let Some(m) = SIAR.find(&s) {
            if out.code.is_none() { out.code = Some(if s.to_lowercase().contains("xpos") { "XPOS".into() } else { "SIAR".into() }); }
            s = s[m.end()..].to_string();
        }
        if let Some(m) = CODE.captures(&s) {
            let c = m[1].to_uppercase();
            let known = BY_CODE.contains_key(c.as_str());
            let looks = m[1].chars().all(|ch| ch.is_ascii_uppercase() || ch == '&') && m[1].len() >= 2;
            if c == "RE" {
                out.is_reply = true;
                s = s[m.get(0).unwrap().end()..].to_string();
            } else if (known || looks) && out.code.is_none() {
                out.code = Some(c);
                s = s[m.get(0).unwrap().end()..].to_string();
            }
        }
        if s == before {
            break;
        }
    }
    out.title = s.trim().to_string();
    let key = PARTS.replace(&out.title.to_lowercase(), "").into_owned();
    out.thread_key = key.split_whitespace().collect::<Vec<_>>().join(" ").trim_matches(|c: char| !c.is_alphanumeric()).to_string();
    out
}

/// The conference topic named by an "(r)" subject: the subject without list tags and "(r)".
pub fn topic_name(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    if let Some(i) = s.find(" Archives: ") {
        if i < 30 {
            s = s[i + 11..].to_string();
        }
    }
    s = s.trim_start_matches('"').to_string();
    s = TRAIL.replace(&s, "").into_owned();
    for _ in 0..3 {
        s = TAG.replace(&s, "").into_owned();
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Rough message type.
pub fn msg_type(sub: &Subject, category: &str, title_lower: &str) -> &'static str {
    if sub.is_reply {
        return "reply";
    }
    let stmt = ["pernyataan", "press release", "siaran pers", "statement", "urgent action", "seruan", "petisi", "petition", "appeal", "maklumat", "open letter", "surat terbuka"];
    if stmt.iter().any(|w| title_lower.contains(w)) {
        return "statement";
    }
    match category {
        "id_daily" | "id_magazine" | "id_online" | "id_wire" | "wire" | "foreign" | "alternative" => "news",
        "government" | "intl_org" => "statement",
        "ngo" => "statement",
        "list_rubric" => "rubric",
        "contributor" => "opinion",
        _ => {
            if ["humor", "lelucon", "joke", "puisi", "poem"].iter().any(|w| title_lower.contains(w)) {
                "literary"
            } else {
                "other"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn body_sources() {
        assert_eq!(code_from_body("According to a November 21 Reuter report by Moses Manoharan datelined Jakarta"), Some("RTR"));
        assert_eq!(code_from_body("Menurut harian Kompas edisi hari ini, pemerintah"), Some("KMP"));
        assert_eq!(code_from_body("The Jakarta Post reported on Monday that"), Some("JKTP"));
        assert_eq!(code_from_body("Students met in Jakarta to discuss the Tempo ban"), None);
    }
    #[test]
    fn subjects() {
        let s = parse_subject("[INDONESIA-L] IN: AFP - Golkar Divided on President's (r)");
        assert_eq!(s.code.as_deref(), Some("AFP"));
        assert!(s.topic_reply);
        assert_eq!(topic_name("[INDONESIA-L] IN: AFP - Golkar Divided on President's (r)"), "IN: AFP - Golkar Divided on President's");
        assert_eq!(topic_name("Issues: Law & Liberty (r)"), "Issues: Law & Liberty");
        assert_eq!(s.title, "Golkar Divided on President's");
        let s = parse_subject("Re: Si Dur - Ulama atau Pezinah?");
        assert!(s.is_reply);
        let s = parse_subject("indonesia-p Archives: [INDONESIA-NEWS] KMP - Pengungsi");
        assert_eq!(s.code.as_deref(), Some("KMP"));
        assert_eq!(s.list_tag.as_deref(), Some("indonesia-news"));
        let s = parse_subject("Purpose Of This Conference (r)");
        assert_eq!(s.code, None);
        assert_eq!(s.title, "Purpose Of This Conference");
    }
}

/// Outlet names as they appear in running text, for messages whose subject carries no code
/// (most messages before 1995 were MacDougall's summaries: "According to a November 21 Reuter report ...").
static NAMES: &[(&str, &str)] = &[
    ("reuter", "RTR"), ("kantor berita antara", "ANTARA"), ("antara news", "ANTARA"), ("majalah tempo", "TEMPO"), ("tempo magazine", "TEMPO"), ("majalah editor", "EDITOR"), ("tabloid detik", "DETIK"), ("agence france", "AFP"), ("afp", "AFP"), ("associated press", "AP"), ("united press", "UPI"), ("upi", "UPI"),
    ("inter press service", "IPS"), ("kyodo", "KYODO"), ("kompas", "KMP"), ("jakarta post", "JKTP"),
    ("suara pembaruan", "PMB"), ("republika", "RPK"), ("media indonesia", "MI"),
    ("jawa pos", "JWP"), ("suara merdeka", "SM"), ("pikiran rakyat", "PR"), ("gatra", "GATRA"), ("forum keadilan", "FORUM"),
    ("sydney morning herald", "SMH"), ("the age", "AGE"), ("far eastern economic review", "FEER"), ("straits times", "ST"),
    ("asiaweek", "ASIAWEEK"), ("asian wall street journal", "AWSJ"), ("wall street journal", "WSJ"), ("new york times", "NYT"),
    ("washington post", "WASHPOST"), ("green left", "GL"), ("tapol", "TAPOL"), ("amnesty", "AI"), ("asia watch", "HRW"),
    ("human rights watch", "HRW"), ("siar", "SIAR"), ("radio australia", "ABC"), ("bbc", "BBC"), ("voice of america", "VOA"),
    ("south china morning post", "SCMP"), ("indonesian observer", "IO"), ("bisnis indonesia", "BI"), ];

static CITE: Lazy<Regex> = Lazy::new(|| {
    let names: Vec<String> = NAMES.iter().map(|(n, _)| regex::escape(n)).collect();
    let alt = names.join("|");
    Regex::new(&format!(
        r"(?i)(?:according to|reported by|reports? (?:in|by|from)|report(?:ed)? in|said|menurut|dikutip dari|dilaporkan|sumber\s*:|source\s*:|\(|datelined|quoted (?:in|by)|in an? (?:article|editorial|report) (?:in|by))\s+(?:a |an |the |harian |majalah |mingguan |tabloid |kantor berita )?(?:[A-Za-z0-9,.' -]{{0,40}}? )?({alt})\b|\b({alt})(?:'s)?\s+(?:reported|reports|said|quoted|melaporkan|memberitakan|news agency|report|correspondent|dispatch)\b"
    )).unwrap()
});

static WIRE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(reuters?|afp|agence france-presse|associated press|upi|kyodo|inter press service)\b").unwrap());

pub fn code_from_body(body: &str) -> Option<&'static str> {
    let head: String = body.chars().take(500).collect();
    let lookup = |n: &str| -> Option<&'static str> {
        let n = n.to_lowercase();
        NAMES.iter().find(|(k, _)| n == *k || n.starts_with(k)).map(|(_, c)| *c)
    };
    if let Some(m) = CITE.captures(&head) {
        if let Some(g) = m.get(1).or_else(|| m.get(2)) {
            return lookup(g.as_str());
        }
    }
    let head300: String = head.chars().take(300).collect();
    WIRE.captures(&head300).and_then(|m| lookup(m.get(1).unwrap().as_str()).or_else(|| if m[1].to_lowercase().starts_with("reuter") { Some("RTR") } else { None }))
}
