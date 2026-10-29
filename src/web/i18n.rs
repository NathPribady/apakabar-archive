//! Two languages: English and Indonesian. The language of a request is chosen by `?lang=`
//! (remembered in a cookie), then the cookie, then the browser's Accept-Language. It is held
//! for the request in a task-local, so templates call `t("English", "Indonesia")` without
//! passing it around. Work moved to blocking threads must read `lang()` before it moves.
//!
//! Numbers and dates follow the language: 137,146 and 14 May 1998 in English, 137.146 and
//! 14 Mei 1998 in Indonesian.

use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Id,
}

impl Lang {
    pub fn parse(s: &str) -> Option<Lang> {
        match s.trim().to_ascii_lowercase().as_str() {
            "en" => Some(Lang::En),
            "id" | "in" => Some(Lang::Id),
            _ => None,
        }
    }
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Id => "id",
        }
    }
    /// The browser's preference: Indonesian when it comes before English, otherwise English.
    pub fn from_accept(h: &str) -> Lang {
        for part in h.split(',') {
            let tag = part.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
            if tag.starts_with("id") || tag.starts_with("in") {
                return Lang::Id;
            }
            if tag.starts_with("en") {
                return Lang::En;
            }
        }
        Lang::En
    }
}

/// What a page needs to know about the request it answers.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub lang: Lang,
    /// path and query without `lang=`, for the language switch
    pub path: String,
    /// the form token of this browser (see security.rs)
    pub csrf: Arc<String>,
}

tokio::task_local! {
    pub static CTX: Ctx;
}

pub fn ctx() -> Option<Ctx> {
    CTX.try_with(|c| c.clone()).ok()
}

pub fn lang() -> Lang {
    CTX.try_with(|c| c.lang).unwrap_or(Lang::En)
}

pub fn is_id() -> bool {
    lang() == Lang::Id
}

/// The English or the Indonesian text, by the language of the request.
pub fn t<'a>(en: &'a str, id: &'a str) -> &'a str {
    if is_id() { id } else { en }
}

/// Same, for formatted text: `tf(|| format!(...), || format!(...))`.
pub fn tf(en: impl FnOnce() -> String, id: impl FnOnce() -> String) -> String {
    if is_id() { id() } else { en() }
}

// ---------------------------------------------------------------- numbers

pub fn fmt_n(n: i64) -> String {
    let sep = if is_id() { '.' } else { ',' };
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// A number with `d` decimals and the decimal mark of the language.
pub fn dec(x: f64, d: usize) -> String {
    let s = format!("{:.*}", d, x);
    if is_id() { s.replace('.', ",") } else { s }
}

/// A share as a percentage: whole numbers from 9.5%, one decimal below.
pub fn pct(x: f64) -> String {
    if x >= 0.095 { format!("{}%", dec(x * 100.0, 0)) } else { format!("{}%", dec(x * 100.0, 1)) }
}

/// A share as a percentage with one decimal.
pub fn pct1(x: f64) -> String {
    format!("{}%", dec(x * 100.0, 1))
}

// ---------------------------------------------------------------- dates

const MONTHS_EN: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTHS_ID: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "Mei", "Jun", "Jul", "Agu", "Sep", "Okt", "Nov", "Des"];
pub const MONTHS_LONG_EN: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const MONTHS_LONG_ID: [&str; 12] = ["Januari", "Februari", "Maret", "April", "Mei", "Juni", "Juli", "Agustus", "September", "Oktober", "November", "Desember"];

/// Short month name, 1 = January.
pub fn month_short(m: usize) -> &'static str {
    let k = m.clamp(1, 12) - 1;
    if is_id() { MONTHS_ID[k] } else { MONTHS_EN[k] }
}

pub fn month_long(m: usize) -> &'static str {
    let k = m.clamp(1, 12) - 1;
    if is_id() { MONTHS_LONG_ID[k] } else { MONTHS_LONG_EN[k] }
}

pub fn weekdays() -> [&'static str; 7] {
    if is_id() { ["Sen", "Sel", "Rab", "Kam", "Jum", "Sab", "Min"] } else { ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] }
}

/// "1998-05-14" -> "14 May 1998" / "14 Mei 1998"; "1998-05" -> "May 1998".
pub fn nice_date(d: &str) -> String {
    if !d.is_ascii() {
        return d.to_string();
    }
    if d.len() >= 10 {
        let m: usize = d[5..7].parse().unwrap_or(1);
        let day: u32 = d[8..10].parse().unwrap_or(1);
        format!("{} {} {}", day, month_long(m), &d[..4])
    } else if d.len() == 7 {
        let m: usize = d[5..7].parse().unwrap_or(1);
        format!("{} {}", month_long(m), &d[..4])
    } else {
        d.to_string()
    }
}

/// "1998-05" -> "May 1998" (short month).
pub fn nice_ym(ym: &str) -> String {
    if !ym.is_ascii() {
        return ym.to_string();
    }
    let m: usize = ym.get(5..7).and_then(|v| v.parse().ok()).unwrap_or(1);
    format!("{} {}", month_short(m), &ym[..4.min(ym.len())])
}

// ---------------------------------------------------------------- labels of the data

/// Indonesian names and descriptions of the themes (the English ones are in lexicon.rs).
pub fn theme_name(slug: &str, en: &str) -> String {
    if !is_id() {
        return en.to_string();
    }
    THEMES_ID.iter().find(|t| t.0 == slug).map(|t| t.1.to_string()).unwrap_or_else(|| en.to_string())
}

pub fn theme_desc(slug: &str, en: &str) -> String {
    if !is_id() {
        return en.to_string();
    }
    THEMES_ID.iter().find(|t| t.0 == slug).map(|t| t.2.to_string()).unwrap_or_else(|| en.to_string())
}

const THEMES_ID: &[(&str, &str, &str)] = &[
    ("east-timor", "Timor Timur", "Pendudukan, perlawanan, referendum 1999 dan sesudahnya"),
    ("aceh", "Aceh", "Provinsi dan konfliknya: operasi militer (DOM), GAM, gerakan referendum, pengungsi dan otonomi"),
    ("papua", "Papua / Irian Jaya", "Wilayah dan penduduknya: OPM dan gerakan kemerdekaan, Freeport dan Timika, operasi militer, kelaparan dan pembangunan"),
    ("communal-conflict", "Konflik komunal", "Maluku, Poso, Kalimantan Barat dan Kalimantan Tengah"),
    ("labour", "Perburuhan", "Buruh, pemogokan, upah, serikat buruh independen dan buruh migran"),
    ("students", "Gerakan mahasiswa", "Aktivisme mahasiswa dan protes kampus, dari tahun 1990-an hingga Reformasi dan sesudahnya"),
    ("press", "Kebebasan pers dan sensor", "Pembredelan dan sensor, wartawan dan keselamatannya, AJI, media dan internet"),
    ("pdi-megawati", "PDI dan Megawati", "Perpecahan PDI 1996, peristiwa 27 Juli 1996 dan PDI-P"),
    ("left-1965", "Kiri dan 1965", "PKI, Gerwani, tahanan politik, PRD dan ingatan tentang 1965"),
    ("islam", "Islam", "Organisasi, politik dan pemikiran Islam: ICMI, NU, Muhammadiyah, gerakan Islamis"),
    ("chinese", "Tionghoa Indonesia", "Diskriminasi, kekerasan anti-Tionghoa, kewarganegaraan"),
    ("military", "Militer", "ABRI/TNI, dwifungsi, Kopassus, komando teritorial"),
    ("human-rights", "Hak asasi manusia", "Pelanggaran seperti penghilangan paksa, penyiksaan dan pembunuhan; Komnas HAM dan kelompok-kelompok HAM; impunitas"),
    ("economy", "Ekonomi dan krisis", "Pertumbuhan, krisis 1997–98, IMF"),
    ("corruption", "Korupsi dan keluarga Soeharto", "KKN, bisnis Cendana, pengadilan"),
    ("elections", "Pemilu, partai dan parlemen", "Pemilu 1992, 1997 dan 1999, partai politik, sidang-sidang DPR dan MPR"),
    ("environment", "Lingkungan hidup", "Kebakaran hutan dan kabut asap, pertambangan, tanah"),
    ("women", "Perempuan dan gender", "Gerakan dan hak perempuan, kekerasan berbasis gender, buruh perempuan, Suara Ibu Peduli"),
    ("may-1998", "Kerusuhan Mei 1998", "Kerusuhan, penjarahan dan kekerasan seksual 13–15 Mei 1998 serta penyelidikannya"),
];

pub fn event_name(slug: &str, en: &str) -> String {
    if !is_id() {
        return en.to_string();
    }
    EVENTS_ID.iter().find(|e| e.0 == slug).map(|e| e.1.to_string()).unwrap_or_else(|| en.to_string())
}

pub fn event_desc(slug: &str, en: &str) -> String {
    if !is_id() {
        return en.to_string();
    }
    EVENTS_ID.iter().find(|e| e.0 == slug).map(|e| e.2.to_string()).unwrap_or_else(|| en.to_string())
}

const EVENTS_ID: &[(&str, &str, &str)] = &[
    ("santa-cruz", "Pembantaian Santa Cruz, Dili", "Tentara menembaki pelayat di pemakaman Santa Cruz, Dili."),
    ("xanana-captured", "Xanana Gusmão ditangkap", "Pemimpin Fretilin itu ditangkap di Dili."),
    ("marsinah", "Marsinah ditemukan terbunuh", "Jenazah aktivis buruh dari Sidoarjo itu ditemukan."),
    ("medan-labour", "Unjuk rasa buruh di Medan", "Unjuk rasa buruh besar-besaran di Medan; para pemimpin SBSI ditangkap."),
    ("press-ban", "Tempo, Editor dan DeTIK dibredel", "Departemen Penerangan mencabut izin terbit tiga majalah mingguan."),
    ("aji", "AJI berdiri", "Para wartawan menandatangani Deklarasi Sirnagalih yang mendirikan Aliansi Jurnalis Independen."),
    ("pdi-medan", "Kongres PDI di Medan menyingkirkan Megawati", "Kongres yang didukung pemerintah mengangkat Soerjadi sebagai ketua PDI."),
    ("27-july", "Penyerbuan kantor PDI", "Kantor Megawati di Jakarta diserbu; kerusuhan menyusul dan PRD dijadikan kambing hitam."),
    ("nobel", "Nobel Perdamaian untuk Belo dan Ramos-Horta", "Komite Nobel menganugerahkan hadiah kepada uskup dan diplomat Timor Timur itu."),
    ("situbondo", "Kerusuhan Situbondo", "Gereja-gereja dibakar di Situbondo, Jawa Timur."),
    ("tasikmalaya", "Kerusuhan Tasikmalaya", "Kerusuhan menyasar toko milik warga Tionghoa dan gereja di Tasikmalaya."),
    ("election-1997", "Pemilu 1997", "Pemilu terakhir Orde Baru, diwarnai kekerasan dan gerakan Mega-Bintang."),
    ("rupiah-float", "Rupiah diambangkan", "Bank Indonesia melepaskan rentang intervensi rupiah."),
    ("imf", "Kesepakatan pertama dengan IMF", "Indonesia menandatangani letter of intent pertamanya dengan IMF."),
    ("soeharto-7th", "Soeharto terpilih untuk ketujuh kalinya", "MPR memilih kembali Soeharto, dengan Habibie sebagai wakil presiden."),
    ("trisakti", "Penembakan Trisakti", "Empat mahasiswa tewas ditembak di Universitas Trisakti."),
    ("may-riots", "Kerusuhan Mei (13–15 Mei)", "Kerusuhan, penjarahan, pembakaran dan kekerasan seksual terhadap warga Tionghoa di Jakarta dan kota-kota lain."),
    ("soeharto-resigns", "Soeharto mundur", "Habibie menjadi presiden."),
    ("biak", "Pembantaian Biak", "Aparat keamanan menembaki pengibaran bendera Bintang Kejora di Biak."),
    ("ita", "Ita Martadinata dibunuh", "Relawan muda yang hendak bersaksi tentang perkosaan Mei itu dibunuh."),
    ("tgpf", "Laporan TGPF tentang Kerusuhan Mei", "Laporan Tim Gabungan Pencari Fakta (bertanggal 23 Oktober, diumumkan awal November)."),
    ("semanggi-1", "Semanggi I", "Tentara menembaki mahasiswa yang memprotes Sidang Istimewa MPR."),
    ("ambon", "Konflik Ambon pecah", "Kekerasan antara warga Kristen dan Muslim pecah di Ambon."),
    ("habibie-timor", "Habibie menawarkan pilihan bagi Timor Timur", "Jakarta mengumumkan bahwa Timor Timur boleh menolak otonomi."),
    ("liquica", "Pembantaian di gereja Liquiçá", "Milisi menyerang para pengungsi yang berlindung di gereja Liquiçá."),
    ("election-1999", "Pemilu 1999", "Pemilu bebas pertama sejak 1955."),
    ("referendum", "Referendum Timor Timur", "78,5% pemilih menolak otonomi; kekerasan milisi menyusul pengumuman hasilnya."),
    ("interfet", "INTERFET mendarat di Dili", "Pasukan pimpinan Australia tiba di Timor Timur."),
    ("gus-dur", "Abdurrahman Wahid terpilih sebagai presiden", "MPR memilih Gus Dur; Megawati menjadi wakil presiden."),
    ("aceh-rally", "Rapat akbar referendum Aceh", "Ratusan ribu orang berkumpul di Banda Aceh menuntut referendum."),
    ("church-bombs", "Bom gereja malam Natal", "Bom meledak serentak di gereja-gereja di beberapa kota."),
    ("sampit", "Kekerasan Sampit", "Pembunuhan terhadap pendatang Madura di Kalimantan Tengah."),
    ("megawati-president", "Wahid dilengserkan; Megawati menjadi presiden", "MPR memakzulkan Wahid dan melantik Megawati."),
    ("sept-11", "Serangan 11 September", "Serangan di Amerika Serikat dan gemanya di Indonesia."),
    ("theys", "Theys Eluay dibunuh", "Pemimpin Papua itu dibunuh sepulang dari jamuan makan malam Kopassus."),
];

/// The kind of outlet, for the record of a message and the source pages.
pub fn category_label(cat: &str) -> String {
    let en = crate::sources::category_label(cat);
    if !is_id() {
        return en.to_string();
    }
    match cat {
        "id_daily" => "Surat kabar harian Indonesia",
        "id_magazine" => "Majalah berita Indonesia",
        "id_online" => "Berita daring Indonesia",
        "id_wire" => "Antara (kantor berita negara)",
        "alternative" => "Pers alternatif dan bawah tanah",
        "wire" => "Kantor berita internasional",
        "foreign" => "Pers dan penyiaran asing",
        "ngo" => "LSM hak asasi manusia dan lainnya",
        "list_rubric" => "Rubrik apakabar",
        "contributor" => "Kontributor tetap",
        "uncoded" => "Tanpa kode sumber",
        "other_code" => "Kode lain (kebanyakan kontributor perorangan)",
        "government" => "Instansi pemerintah",
        "intl_org" => "Organisasi internasional",
        _ => "Kode lain",
    }
    .to_string()
}

pub fn lang_label(l: &str) -> String {
    match l {
        "id" => t("Indonesian", "bahasa Indonesia").into(),
        "en" => t("English", "bahasa Inggris").into(),
        "mixed" => t("Indonesian and English", "bahasa Indonesia dan Inggris").into(),
        "und" => t("undetermined", "tidak dapat ditentukan").into(),
        _ => l.strip_prefix("other:").unwrap_or(l).to_string(),
    }
}

/// The type of a message, as the subject line or the text shows it.
pub fn type_label(t0: &str) -> String {
    match t0 {
        "news" => t("news", "berita"),
        "reply" => t("reply", "balasan"),
        "opinion" => t("opinion", "opini"),
        "statement" => t("statement", "pernyataan"),
        "rubric" => t("rubric", "rubrik"),
        "literary" => t("literary", "sastra"),
        "other" => t("other", "lain-lain"),
        x => x,
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with<T>(l: Lang, f: impl FnOnce() -> T) -> T {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(CTX.scope(Ctx { lang: l, path: "/".into(), csrf: Arc::new(String::new()) }, async move { f() }))
    }

    #[test]
    fn numbers_and_dates() {
        assert_eq!(with(Lang::En, || fmt_n(137146)), "137,146");
        assert_eq!(with(Lang::Id, || fmt_n(137146)), "137.146");
        assert_eq!(with(Lang::En, || pct1(0.9948)), "99.5%");
        assert_eq!(with(Lang::Id, || pct1(0.9948)), "99,5%");
        assert_eq!(with(Lang::Id, || nice_date("1998-05-14")), "14 Mei 1998");
        assert_eq!(with(Lang::En, || nice_date("1998-05-14")), "14 May 1998");
        assert_eq!(fmt_n(1500), "1,500"); // outside a request: English
        assert_eq!(Lang::from_accept("id-ID,id;q=0.9,en-US;q=0.8"), Lang::Id);
        assert_eq!(Lang::from_accept("en-GB,en;q=0.9,id;q=0.8"), Lang::En);
        assert_eq!(Lang::from_accept("fr-FR"), Lang::En);
    }

    #[test]
    fn every_theme_and_event_has_an_indonesian_name() {
        for t in crate::lexicon::THEMES {
            assert!(THEMES_ID.iter().any(|x| x.0 == t.slug), "{}", t.slug);
        }
        for e in crate::lexicon::EVENTS {
            assert!(EVENTS_ID.iter().any(|x| x.0 == e.slug), "{}", e.slug);
        }
    }
}
