//! Matching the themes and names of the lexicon in a message.
//!
//! Themes are found in two steps.
//!
//! 1. A message *mentions* a theme when it has one of the theme's terms and two theme words
//!    in all, or a term in its title, or weak words and context words in one paragraph (three
//!    hits in all). Set phrases ("tanah air", "siaran pers", "Jakarta Post") are matched as a
//!    whole and ignored. Web and e-mail addresses are blanked first.
//! 2. A message is *about* a theme when, in addition, a small logistic model judges from the
//!    pattern of the matches (in the title, in the opening paragraphs, their density and spread,
//!    the number of other themes) that the theme is a real subject of the text. The model was
//!    fitted on 936 messages read for the purpose (see `model.rs`); the share of "about"
//!    messages that really are about the theme is measured on fresh samples.

use crate::lexicon;
use aho_corasick::{AhoCorasick, MatchKind};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Strong(usize),
    Weak(usize),
    Context(usize),
    Stop,
}

static URLS: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(?:https?://|www\.)\S+|\b[\w.+-]+@[\w-]+(?:\.[\w-]+)+\b").unwrap());
static DIGEST: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)warta berita|\bnews\b.*#|^news \d|digest|kliping|berita #|update|ringkasan|cuplikan|headlines|berita singkat|news summary|\bnb ?news|journal - ").unwrap()
});

/// Web addresses and e-mail addresses carry words ("geocities.com/RainForest", "tapol@gn.apc.org")
/// that are not about anything in the message; they are blanked before matching.
pub fn without_addresses(s: &str) -> std::borrow::Cow<'_, str> {
    if s.contains("://") || s.contains("www.") || s.contains('@') {
        URLS.replace_all(s, " ")
    } else {
        std::borrow::Cow::Borrowed(s)
    }
}

pub fn is_digest(title: &str) -> bool {
    DIGEST.is_match(title)
}

/// How a theme's words fall in one message.
#[derive(Debug, Clone, Default)]
pub struct ThemeHits {
    pub theme: usize,
    /// terms and weak words in the title
    pub ts: u32,
    pub tw: u32,
    /// terms, weak words and context words in the text
    pub s: u32,
    pub w: u32,
    pub c: u32,
    /// hits in the first two paragraphs (context words count half)
    pub lead: f64,
    /// paragraphs with a term or weak word; their number; the most hits in one paragraph
    pub ppar: u32,
    pub paras: u32,
    pub maxpar: f64,
    pub words: u32,
    /// a weak word and a context word in the same paragraph
    pub together: bool,
}

impl ThemeHits {
    pub fn mentions(&self) -> bool {
        let strong = self.s + self.ts;
        (strong >= 1 && strong + self.w + self.tw >= 2) || self.ts >= 1 || (self.together && self.w + self.c >= 3)
    }
    /// all hits, for sorting
    pub fn hits(&self) -> u32 {
        self.s + self.w + self.ts + self.tw
    }
    /// The model's inputs, in the order of `model::WEIGHTS`.
    pub fn features(&self, nthemes: usize, digest: bool) -> Vec<f64> {
        let hits = self.s as f64 + 0.5 * self.w as f64 + 0.5 * self.c as f64;
        let dens = hits / (self.words.max(60) as f64) * 1000.0;
        let psh = self.ppar as f64 / self.paras.max(1) as f64;
        let mut v = vec![
            if self.ts > 0 { 1.0 } else { 0.0 },
            (self.tw as f64).ln_1p(),
            self.lead.ln_1p(),
            dens.ln_1p(),
            psh,
            hits.ln_1p(),
            (self.s as f64).ln_1p(),
            (self.words as f64).ln_1p(),
            (self.ppar as f64).ln_1p(),
            self.maxpar.ln_1p(),
            (nthemes as f64).ln_1p(),
            if digest { 1.0 } else { 0.0 },
        ];
        for (i, _) in lexicon::THEMES.iter().enumerate() {
            v.push(if i == self.theme { 1.0 } else { 0.0 });
        }
        v
    }
}

fn boundary_ok(hay: &[u8], s: usize, e: usize) -> bool {
    let before = s == 0 || !(hay[s - 1].is_ascii_alphanumeric());
    let after = e >= hay.len() || !(hay[e].is_ascii_alphanumeric());
    before && after
}

/// A place name used as a dateline or a credit is not a mention of the place: at the start of
/// a line and followed by "(" "," ":" or a dash ("JAKARTA (KOMPAS) -", "Washington, May 3",
/// "WASHINGTON: The"); in capitals and followed by an outlet in brackets or a date, wherever it
/// stands ("Harga Mati MEDAN (Waspada):", "By Jim Wolf WASHINGTON, Dec 16"); or alone in
/// brackets after a reporter's name ("Supriadi Gunawan (Medan)").
fn is_dateline(hay: &[u8], s: usize, e: usize) -> bool {
    let mut k = s;
    while k > 0 && (hay[k - 1] == b' ' || hay[k - 1] == b'>') {
        k -= 1;
    }
    let at_start = k == 0 || hay[k - 1] == b'\n';
    let mut j = e;
    while j < hay.len() && hay[j] == b' ' {
        j += 1;
    }
    let next = hay.get(j).copied();
    if at_start && matches!(next, Some(b'(') | Some(b',') | Some(b'-') | Some(b':') | Some(0xE2)) {
        return true;
    }
    if s > 0 && hay[s - 1] == b'(' && hay.get(e) == Some(&b')') {
        return true;
    }
    let caps = hay[s..e].iter().all(|b| !b.is_ascii_lowercase()) && e - s >= 4;
    if caps && next == Some(b'(') {
        return true;
    }
    if caps && next == Some(b',') {
        // a date follows: "WASHINGTON, Dec 16", "JAKARTA, 3 Mei"
        let rest = &hay[j + 1..hay.len().min(j + 14)];
        let rest = String::from_utf8_lossy(rest);
        let r = rest.trim_start();
        return r.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
            || ["jan", "feb", "mar", "apr", "may", "mei", "jun", "jul", "aug", "agu", "sep", "oct", "okt", "nov", "nop", "dec", "des"].iter().any(|m| r.to_ascii_lowercase().starts_with(m));
    }
    false
}

pub struct Tagger {
    ac_t: AhoCorasick,
    thits: Vec<Vec<Hit>>,
    ac_e: AhoCorasick,
    ecase: Vec<bool>,
    ehits: Vec<Vec<Option<usize>>>,
    eplace: Vec<bool>,
}

impl Tagger {
    pub fn new() -> Tagger {
        // one pattern per distinct phrase; a phrase can be a term of one theme and a stop of another
        let mut tmap: BTreeMap<String, Vec<Hit>> = BTreeMap::new();
        for (i, t) in lexicon::THEMES.iter().enumerate() {
            for w in t.terms { tmap.entry(w.to_string()).or_default().push(Hit::Strong(i)); }
            for w in t.weak { tmap.entry(w.to_string()).or_default().push(Hit::Weak(i)); }
            for w in t.stops { tmap.entry(w.to_string()).or_default().push(Hit::Stop); }
            for w in t.context { tmap.entry(w.to_string()).or_default().push(Hit::Context(i)); }
        }
        let tpats: Vec<String> = tmap.keys().cloned().collect();
        let thits: Vec<Vec<Hit>> = tmap.into_values().collect();
        let mut emap: BTreeMap<String, Vec<Option<usize>>> = BTreeMap::new();
        for (i, e) in lexicon::ENTITIES.iter().enumerate() {
            for a in e.aliases { emap.entry(a.to_string()).or_default().push(Some(i)); }
        }
        for w in lexicon::NAME_STOPS { emap.entry(w.to_string()).or_default().push(None); }
        let epats: Vec<String> = emap.keys().cloned().collect();
        let ecase: Vec<bool> = epats.iter().map(|p| lexicon::CASE_SENSITIVE.contains(&p.as_str())).collect();
        let ehits: Vec<Vec<Option<usize>>> = emap.into_values().collect();
        let eplace: Vec<bool> = ehits.iter().map(|v| v.iter().flatten().any(|&i| lexicon::ENTITIES[i].kind == "place")).collect();
        Tagger {
            ac_t: AhoCorasick::builder().match_kind(MatchKind::LeftmostLongest).ascii_case_insensitive(true).build(&tpats).expect("theme patterns"),
            thits,
            ac_e: AhoCorasick::builder().match_kind(MatchKind::LeftmostLongest).ascii_case_insensitive(true).build(&epats).expect("name patterns"),
            ecase,
            ehits,
            eplace,
        }
    }

    /// Themes the message mentions (with how their words fall) and the names it contains.
    pub fn tag(&self, title: &str, body: &str) -> (Vec<ThemeHits>, Vec<(usize, u32)>) {
        let n = lexicon::THEMES.len();
        let body = without_addresses(body);
        let body: &str = body.as_ref();
        let mut th: Vec<ThemeHits> = (0..n).map(|i| ThemeHits { theme: i, ..Default::default() }).collect();
        let mut last_weak = vec![usize::MAX; n];
        let mut last_ctx = vec![usize::MAX; n];
        // paragraph bookkeeping
        let breaks: Vec<usize> = body.match_indices("\n\n").map(|(k, _)| k).collect();
        let npar = body.split("\n\n").filter(|p| !p.trim().is_empty()).count() as u32;
        let words = body.split_whitespace().count() as u32;
        let mut per_par: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); breaks.len() + 1];
        for (hay, in_title) in [(title, true), (body, false)] {
            let hb = hay.as_bytes();
            for m in self.ac_t.find_iter(hay) {
                if !boundary_ok(hb, m.start(), m.end()) { continue; }
                let par = if in_title { usize::MAX - 1 } else { breaks.partition_point(|&b| b < m.start()) };
                for h in &self.thits[m.pattern().as_usize()] {
                    match *h {
                        Hit::Strong(i) => {
                            if in_title { th[i].ts += 1 } else { th[i].s += 1; *per_par[par].entry(i).or_default() += 1.0; }
                        }
                        Hit::Weak(i) => {
                            if in_title { th[i].tw += 1 } else { th[i].w += 1; *per_par[par].entry(i).or_default() += 1.0; }
                            last_weak[i] = par;
                            if last_ctx[i] == par { th[i].together = true; }
                        }
                        Hit::Context(i) => {
                            if !in_title { th[i].c += 1; *per_par[par].entry(i).or_default() += 0.5; }
                            last_ctx[i] = par;
                            if last_weak[i] == par { th[i].together = true; }
                        }
                        Hit::Stop => {}
                    }
                }
            }
        }
        // paragraph spread (context words alone do not make a paragraph count)
        for (k, m) in per_par.iter().enumerate() {
            for (&i, &h) in m {
                if h >= 1.0 {
                    th[i].ppar += 1;
                }
                if h > th[i].maxpar {
                    th[i].maxpar = h;
                }
                if k <= 1 {
                    th[i].lead += h;
                }
            }
        }
        for t in th.iter_mut() {
            t.paras = npar;
            t.words = words;
        }
        let mut en: BTreeMap<usize, u32> = BTreeMap::new();
        for (hay, in_title) in [(title, true), (body, false)] {
            let hb = hay.as_bytes();
            for m in self.ac_e.find_iter(hay) {
                let k = m.pattern().as_usize();
                if !boundary_ok(hb, m.start(), m.end()) { continue; }
                if self.ecase[k] && !hb[m.start()].is_ascii_uppercase() { continue; }
                if !in_title && self.eplace[k] && is_dateline(hb, m.start(), m.end()) { continue; }
                for e in self.ehits[k].iter().flatten() {
                    *en.entry(*e).or_insert(0) += 1;
                }
            }
        }
        (th.into_iter().filter(|t| t.mentions()).collect(), en.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stops_context_datelines() {
        let t = Tagger::new();
        let slug = |i: usize| lexicon::THEMES[i].slug;
        let (th, _) = t.tag("Cinta tanah air", "Kami cinta tanah air. Tanah air kita indah.");
        assert!(!th.iter().any(|h| slug(h.theme) == "environment"));
        let (th, _) = t.tag("Berita", "Ambon kembali rusuh. Pengungsi memenuhi gereja dan masjid.\n\nLain hal.");
        assert!(th.iter().any(|h| slug(h.theme) == "communal-conflict"));
        let (th, _) = t.tag("Berita", "Gubernur Maluku meresmikan pasar.\n\nRibuan pengungsi di Aceh.\n\nKerusuhan di Jakarta.");
        assert!(!th.iter().any(|h| slug(h.theme) == "communal-conflict"));
        let (_, en) = t.tag("x", "JAKARTA (KOMPAS) - Presiden berpidato.\n\nWASHINGTON (Reuters) - The US said.");
        let names: Vec<&str> = en.iter().map(|(i, _)| lexicon::ENTITIES[*i].slug).collect();
        assert!(!names.contains(&"jakarta") && !names.contains(&"washington"), "{names:?}");
        let (_, en) = t.tag("x", "Demonstrasi di Jakarta kemarin. The Jakarta Post wrote.");
        assert_eq!(en.iter().find(|(i, _)| lexicon::ENTITIES[*i].slug == "jakarta").map(|x| x.1), Some(1));
        let (th, _) = t.tag("x", "Lihat http://www.geocities.com/RainForest/2958 dan tapol@gn.apc.org untuk kabar.");
        assert!(th.is_empty());
        // datelines and credits in the middle of a line, and namesakes
        let ent = |title: &str, body: &str, slug: &str| -> u32 {
            let (_, en) = t.tag(title, body);
            en.iter().find(|(i, _)| lexicon::ENTITIES[*i].slug == slug).map(|x| x.1).unwrap_or(0)
        };
        assert_eq!(ent("x", "Merah Putih Harga Mati MEDAN (Waspada): Dalam rangka HUT.", "medan"), 0);
        assert_eq!(ent("x", "By Jim Wolf WASHINGTON, Dec 16 (Reuter) - Repression rose.", "washington"), 0);
        assert_eq!(ent("x", "WASHINGTON: The Clinton administration has set up a group.", "washington"), 0);
        assert_eq!(ent("x", "Supriadi Gunawan (Medan), M. Harsadi (Pekanbaru).", "medan"), 0);
        assert_eq!(ent("x", "Bus di jalan Banda Aceh-Medan dihentikan. Artileri Medan menembak.", "medan"), 0);
        assert_eq!(ent("x", "University of Washington, Seattle. USIS Washington File.", "washington"), 0);
        assert_eq!(ent("x", "Ribuan buruh di Medan mogok. Pejabat di Washington menolak.", "medan"), 1);
        assert_eq!(ent("x", "Ribuan buruh di Medan mogok. Pejabat di Washington menolak.", "washington"), 1);
        // the May 1998 riots cannot be the subject of a message written before them
        assert!(!lexicon::theme_possible("may-1998", Some("1998-04-16")));
        assert!(lexicon::theme_possible("may-1998", Some("1998-05-14")));
        assert!(lexicon::theme_possible("aceh", Some("1991-01-01")));
    }
}
