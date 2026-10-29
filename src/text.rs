//! Tokenising, stop words, a light Indonesian stemmer and language identification.

use ahash::{AHashMap, AHashSet};
use once_cell::sync::Lazy;

pub static STOP_ID: Lazy<AHashSet<&'static str>> = Lazy::new(|| {
    "yang dan di dengan tidak ini itu untuk dari dalam akan pada ke juga oleh karena bahwa sudah ada mereka kita kami saya \
     adalah atau sebagai telah bisa dapat lebih tersebut harus kata seperti saat masih hanya para sejak namun tetapi tapi \
     serta agar jika kalau lagi pun maka bagi antara setelah belum ia dia nya anda kami kamu kalian apa siapa bagaimana \
     mengapa kenapa ketika sementara secara terhadap hingga sampai menjadi jadi sangat sekarang yaitu yakni demikian \
     sedang akan saja lain pula baru baik semua banyak tak tanpa per atas bawah menurut tentang selama begitu sehingga \
     itulah inilah sendiri beberapa sebuah seorang satu dua tiga pak bapak ibu bu sdr hal olah lalu kemudian dll dsb \
     maupun yg dgn utk dr tdk sdh krn pd spt ttg jg bhw org thn".split_whitespace().collect()
});

pub static STOP_EN: Lazy<AHashSet<&'static str>> = Lazy::new(|| {
    "the and of to is in that for with was are this be on by as it from have has not were at an or which but their they \
     its had been will would there who his her he she we you our said also more than about after into them can could \
     these those when what all one two new other some such only over most may any very no if so up out do does did \
     being i my me us were should then just like how where while because many much well since before under between \
     against during without within through per mr ms dr re fw fwd".split_whitespace().collect()
});

pub fn is_stop(w: &str) -> bool {
    STOP_ID.contains(w) || STOP_EN.contains(w)
}

/// Lowercase word tokens (letters and digits, keeps internal hyphens).
pub fn tokens(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '\''))
        .map(|w| w.trim_matches(|c| c == '-' || c == '\'').to_lowercase())
        .filter(|w| !w.is_empty() && w.len() < 40)
}

/// Language by stop-word counts; whatlang for the rest.
pub fn detect_lang(text: &str) -> String {
    let sample: String = text.chars().take(6000).collect();
    let (mut id, mut en) = (0usize, 0usize);
    for w in tokens(&sample) {
        if STOP_ID.contains(w.as_str()) && !STOP_EN.contains(w.as_str()) {
            id += 1;
        } else if STOP_EN.contains(w.as_str()) && !STOP_ID.contains(w.as_str()) {
            en += 1;
        }
    }
    let tot = id + en;
    if tot >= 8 {
        let fid = id as f64 / tot as f64;
        return if fid >= 0.75 {
            "id".into()
        } else if fid <= 0.25 {
            "en".into()
        } else {
            "mixed".into()
        };
    }
    match whatlang::detect(&sample) {
        Some(info) if info.confidence() > 0.5 => match info.lang() {
            whatlang::Lang::Eng => "en".into(),
            whatlang::Lang::Ind => "id".into(),
            l => format!("other:{}", l.code()),
        },
        _ => "und".into(),
    }
}

/// Indonesian stemming with a root-word dictionary (Sastrawi: Nazief-Adriani rules with
/// Enhanced Confix Stripping, via the `sastrawi-rs` crate). An affix is removed only when a
/// real root word remains, so "perang" and "keterangan" no longer collapse into "ang".
/// Words the dictionary cannot resolve are left as they are; English words pass through.
static DICT: Lazy<sastrawi::Dictionary> = Lazy::new(sastrawi::Dictionary::new);
static STEMMER: Lazy<sastrawi::Stemmer<'static>> = Lazy::new(|| sastrawi::Stemmer::new(&DICT));

/// Corrections for words the dictionary stemmer gets wrong in this corpus.
static OVERRIDES: Lazy<AHashMap<&'static str, &'static str>> = Lazy::new(|| {
    [
        ("pemerkosaan", "perkosa"), ("pemerkosa", "perkosa"), ("pemerkosanya", "perkosa"), ("pemerkosaannya", "perkosa"),
        ("memerkosa", "perkosa"), ("diperkosanya", "perkosa"), ("perkosaannya", "perkosa"),
        ("penculikan", "culik"), ("penculiknya", "culik"), ("penculikannya", "culik"),
        ("kekerasan", "keras"), ("kekerasannya", "keras"),
        ("pemilu", "pemilu"), ("pemilunya", "pemilu"),
        ("pengakuan", "aku"), ("pengakuannya", "aku"),
        ("pemerintah", "pemerintah"), ("pemerintahan", "pemerintah"), ("pemerintahnya", "pemerintah"),
        ("berita", "berita"), ("beritanya", "berita"), ("perempuan", "perempuan"), ("perempuannya", "perempuan"),
        ("tapol", "tapol"), ("napol", "napol"), ("orba", "orba"),
    ]
    .into_iter()
    .collect()
});

thread_local! {
    static CACHE: std::cell::RefCell<AHashMap<String, String>> = std::cell::RefCell::new(AHashMap::new());
}

pub fn stem(w: &str) -> String {
    let w = w.to_lowercase();
    if w.chars().count() <= 3 || w.chars().any(|c| c.is_ascii_digit()) || w.contains('\'') || w.contains('-') {
        return w;
    }
    if let Some(o) = OVERRIDES.get(w.as_str()) {
        return o.to_string();
    }
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&w).cloned()) {
        return hit;
    }
    let s = STEMMER.stem_word(&w).into_owned();
    let s = if s.chars().count() < 3 { w.clone() } else { s };
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 400_000 {
            c.clear();
        }
        c.insert(w, s.clone());
    });
    s
}

/// Stemmed token string for the search index.
pub fn stems(text: &str) -> String {
    let mut out = String::with_capacity(text.len() / 2);
    for t in tokens(text) {
        if is_stop(&t) || t.len() < 3 {
            continue;
        }
        let st = stem(&t);
        if st != t {
            out.push_str(&st);
            out.push(' ');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stemming() {
        assert_eq!(stem("perkosaan"), "perkosa");
        assert_eq!(stem("diperkosa"), "perkosa");
        assert_eq!(stem("memperkosa"), "perkosa");
        assert_eq!(stem("pemerkosaan"), "perkosa");
        assert_eq!(stem("mahasiswa"), "mahasiswa");
        assert_eq!(stem("kerusuhan"), "rusuh");
        assert_eq!(stem("menuntut"), "tuntut");
        assert_eq!(stem("pembantaian"), "bantai");
        // the old rule-only stemmer turned all of these into fragments ("ang", "perin", "ita", "empu")
        assert_eq!(stem("perang"), "perang");
        assert_eq!(stem("peperangan"), "perang");
        assert_eq!(stem("keterangan"), "terang");
        assert_eq!(stem("pemerintah"), "pemerintah");
        assert_eq!(stem("berita"), "berita");
        assert_eq!(stem("perempuan"), "perempuan");
        assert_eq!(stem("penculikan"), "culik");
        assert_eq!(stem("kekerasan"), "keras");
        assert_eq!(stem("pemilu"), "pemilu");
        // English and unknown words pass through
        assert_eq!(stem("elections"), "elections");
        assert_eq!(stem("tionghoa"), "tionghoa");
    }
    #[test]
    fn lang() {
        assert_eq!(detect_lang("Kini mahasiswa pun demo di luar kampus dan mereka tidak akan berhenti untuk itu yang"), "id");
        assert_eq!(detect_lang("The students said that they will continue the protest in the capital and this is the"), "en");
    }
}
