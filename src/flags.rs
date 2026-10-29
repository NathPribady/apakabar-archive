//! Flags for sexual violence and for the May 1998 riots.
//!
//! The first version flagged any message from 13 May 1998 on that mentioned rape anywhere and
//! May, a riot or Chinese Indonesians anywhere else. Read in a sample, most of those messages
//! were about something else (East Timor, Aceh, Ambon, national politics), or used "perkosa"
//! as a figure of speech ("memperkosa hukum", to violate the law). The flag now has tiers:
//!
//! 1. mention: the message mentions rape or sexual violence (figures of speech left out);
//! 2. May 1998: in at least one paragraph the violence is tied to May 1998: the date, the
//!    riots of 13-15 May, the inquiries (TGPF, Tim Relawan, Komnas Perempuan, Ita Martadinata),
//!    or riots and Chinese Indonesians together; and such paragraphs are at least as many as
//!    those about rape elsewhere (East Timor, Aceh, Papua, Maluku, migrant workers abroad);
//! 3. focused: tied to May 1998 as above, and the sexual violence is what the text is about:
//!    twelve or more mentions per thousand words, in a quarter of its paragraphs or more, or
//!    eight per thousand under a title that names it. Texts whose title names East Timor, Aceh
//!    or another place of the list above are left at tier 2. (Rule set after the second
//!    precision check, October 2026: the earlier rule, which counted paragraphs, took the TGPF
//!    report and other texts on the riots in general for focused ones.)
//!
//! Tiers apply from 13 May 1998; before that date only "mention" is recorded (as tier 0).

use once_cell::sync::Lazy;
use regex::Regex;

/// Rape and sexual violence, in Indonesian and English.
pub static SV: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)perkosa\w*|pemerkosa\w*|kekerasan sexual|pelecehan sexual|\brape[ds]?\b|\braping\b|\brapists?\b|sexual(?:ly)? (?:violence|violated|assault(?:ed)?|abuse[ds]?|harass(?:ment|ed)?|torture)|kekerasan seksual|pelecehan seksual|penganiayaan seksual|penyiksaan seksual|gang.rape|mass rapes?").unwrap()
});

/// "memperkosa hukum", "perkosaan terhadap demokrasi", "the rape of justice": figures of speech.
static METAPHOR: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*(?:terhadap\s+|atas\s+)?(?:hak|hukum|demokrasi|konstitusi|keadilan|kebenaran|kedaulatan|undang|uud|uu\b|nilai|aturan|prinsip|kebebasan|akal|kemerdekaan|reformasi|pancasila|agama|sejarah|fakta|kenyataan|bahasa|logika|kata|makna|aspirasi|suara|kehendak|nurani|hati nurani|rakyat|bangsa|negara|kekuasaan|of (?:democracy|justice|the law|the constitution|truth|the nation|the people))").unwrap()
});

/// May 1998 itself: the dates, the riots by name, the inquiries.
static MAY_SPECIFIC: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:mei|may)\s*'?(?:19)?98\b|\b1[345]\s*(?:-|–|s/d|sampai|hingga|dan)\s*1[45]\s*(?:mei|may)\b|\b1[345]\s+(?:mei|may)\b|\bmay\s+1[345]\b|mid-may|pertengahan mei|kerusuhan mei|tragedi mei|peristiwa mei|huru-hara mei|may riots|may tragedy|may violence|may unrest|\btgpf\b|tim gabungan pencari fakta|fact-finding team|tim relawan|volunteers?'? team|team of volunteers|volunteers for humanity|martadinata|komnas perempuan|commission on violence against women|glodok").unwrap()
});
static RIOT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)kerusuhan|\briots?\b|\brioting\b|\brioters\b|huru.hara|penjarahan|\blooting\b|\blooters?\b|amuk massa").unwrap());
static CHINESE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)tionghoa|\bcina\b|\bchinese\b|keturunan|\betnis\b|\bethnic\b|non.?pribumi|sino-indonesian").unwrap());

/// Other places and settings where rape was widely reported in these years.
static ELSEWHERE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)timor|timtim|\bdili\b|aceh|\bdom\b|papua|irian|maluku|ambon|kalimantan|sampit|sambas|\bposo\b|liquica|suai|atambua|bosnia|kosovo|rwanda|bangladesh|\barab|saudi|malaysia|kuwait|hong kong|\btk[iw]\b|migrant").unwrap()
});

pub const MAY_START: &str = "1998-05-13";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags {
    pub sv: bool,
    /// 0 none, 1 mention, 2 tied to May 1998, 3 focused (only from 13 May 1998)
    pub may_tier: u8,
}

/// Mentions of sexual violence that are not figures of speech.
fn real_sv(text: &str) -> usize {
    SV.find_iter(text)
        .filter(|m| {
            let mut end = (m.end() + 40).min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            !METAPHOR.is_match(&text[m.end()..end])
        })
        .count()
}

pub fn flags(title: &str, text: &str, date: Option<&str>) -> Flags {
    let in_title = real_sv(title) > 0;
    let sv = in_title || real_sv(text) > 0;
    if !sv {
        return Flags::default();
    }
    if !date.map(|d| d >= MAY_START).unwrap_or(false) {
        return Flags { sv, may_tier: 0 };
    }
    let paras: Vec<&str> = text.split("\n\n").filter(|p| !p.trim().is_empty()).collect();
    let (mut tied, mut elsewhere, mut sv_paras, mut sv_n) = (0usize, 0usize, 0usize, 0usize);
    for p in &paras {
        let k = real_sv(p);
        if k == 0 {
            continue;
        }
        sv_paras += 1;
        sv_n += k;
        if MAY_SPECIFIC.is_match(p) || (RIOT.is_match(p) && CHINESE.is_match(p)) {
            tied += 1;
        } else if ELSEWHERE.is_match(p) {
            elsewhere += 1;
        }
    }
    let n = paras.len().max(1) as f64;
    let words = text.split_whitespace().count().max(60) as f64;
    let density = sv_n as f64 / words * 1000.0;
    let spread = sv_paras as f64 / n;
    let focused = elsewhere < tied
        && !ELSEWHERE.is_match(title)
        && ((density >= 12.0 && spread >= 0.25) || (in_title && density >= 8.0));
    let tier = if tied == 0 || elsewhere > tied {
        1
    } else if focused {
        3
    } else {
        2
    };
    Flags { sv, may_tier: tier }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tiers() {
        let f = flags("Aceh", "Tentara memperkosa warga di Pidie.\n\nKorban melapor ke Komnas HAM.", Some("1998-08-01"));
        assert_eq!(f, Flags { sv: true, may_tier: 1 });
        let f = flags("Berita", "Kerusuhan 13-14 Mei menelan korban. Banyak perempuan Tionghoa diperkosa.\n\nLain hal.\n\nSatu.\n\nDua.\n\nTiga.\n\nEmpat.", Some("1998-06-01"));
        assert_eq!(f.may_tier, 2);
        let f = flags("Tim Relawan: Laporan Perkosaan", "Tim Relawan mencatat 168 korban perkosaan dalam kerusuhan Mei.", Some("1998-07-14"));
        assert_eq!(f.may_tier, 3);
        // the riots in general, with the rapes as one point among many: tier 2
        let long = format!("Kerusuhan 13-14 Mei dan perkosaan terhadap perempuan Tionghoa.\n\n{}", "TGPF memeriksa para jenderal tentang pertemuan di Makostrad dan penembakan mahasiswa.\n\n".repeat(12));
        assert_eq!(flags("Laporan TGPF", &long, Some("1998-11-05")).may_tier, 2);
        // East Timor in the title: not focused on May 1998
        let f = flags("Timor Rape Victims Too Afraid", "Rape victims in Dili were afraid, as rape victims of the May 13-14 riots were.", Some("1999-10-01"));
        assert!(f.may_tier <= 2);
        let f = flags("Perkosaan", "Kerusuhan Mei: perempuan Tionghoa diperkosa.\n\nDi Timor Timur tentara memperkosa.\n\nDi Aceh juga ada perkosaan.", Some("1998-07-14"));
        assert_eq!(f.may_tier, 1);
        let f = flags("Hukum", "Mereka memperkosa hukum dan memperkosa demokrasi sejak Mei 1998.", Some("1998-07-14"));
        assert_eq!(f, Flags::default());
        let f = flags("Old", "rape in Dili", Some("1995-01-01"));
        assert_eq!(f, Flags { sv: true, may_tier: 0 });
        assert_eq!(flags("x", "no violence here", Some("1998-06-01")), Flags::default());
    }
}
