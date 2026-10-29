//! Page shell and stylesheet. No JavaScript frameworks and no web fonts: pages are
//! server-rendered HTML with inline SVG charts, a few kilobytes each.

use super::auth::User;
use super::i18n::{self, t};
use maud::{html, Markup, DOCTYPE};

pub use super::i18n::{fmt_n, nice_date, pct};

static CONTACTS: once_cell::sync::OnceCell<Vec<String>> = once_cell::sync::OnceCell::new();

/// The editors' addresses (from --contact: one or more, separated by commas, spaces or "/").
pub fn set_contacts(list: &str) {
    let v: Vec<String> = list.split([',', ';', ' ', '/']).map(|s| s.trim().to_string()).filter(|s| s.contains('@') && !s.starts_with('@') && !s.ends_with('@')).collect();
    let _ = CONTACTS.set(v);
}

pub fn contacts() -> &'static [String] {
    CONTACTS.get().map(|v| v.as_slice()).unwrap_or(&[])
}

/// "a@x or b@y" as mail links.
pub fn contact_links() -> Markup {
    html! { @for (i, c) in contacts().iter().enumerate() { @if i > 0 { (t(" or ", " atau ")) } a href=(format!("mailto:{c}")) { (c) } } }
}

/// A short fingerprint of the stylesheet: pages link `/style.css?v=<it>`, so the file can be
/// cached for a year and a new stylesheet is fetched as soon as it changes.
pub fn css_version() -> &'static str {
    static V: once_cell::sync::Lazy<String> = once_cell::sync::Lazy::new(|| crate::site::sha256_hex(CSS)[..10].to_string());
    &V
}

pub const CSS: &str = r#"
:root{--bg:#fbfaf7;--paper:#ffffff;--ink:#1d1b18;--ink2:#5b5750;--ink3:#6b665e;--grey:#8a857c;--rule:#e4e0d8;--accent:#9b2c1f;--accent2:#1f5f8b;
--id:#9b2c1f;--en:#1f5f8b;--mixed:#c08a2b;--other:#b9b4aa;--hl:#fff1b8;--chip:#f1eee8;--teal:#2a8c82;--plum:#7d4e8c;--sky:#7fadd3;
--serif:"Iowan Old Style","Palatino Linotype",Palatino,"Book Antiqua",Georgia,serif;--sans:system-ui,-apple-system,"Segoe UI",Roboto,"Helvetica Neue",Arial,sans-serif;--mono:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
@media (prefers-color-scheme:dark){:root:not([data-theme=light]){--bg:#15140f;--paper:#1c1b16;--ink:#ece8df;--ink2:#b5afa3;--ink3:#958f84;--grey:#857f74;--rule:#34312a;--accent:#e07a5f;--accent2:#7fb3d5;--id:#e07a5f;--en:#7fb3d5;--mixed:#e0b65f;--other:#5d594f;--hl:#4a3f12;--chip:#26241e;--teal:#5cc4b8;--plum:#b98ccb;--sky:#3e77a3}}
*{box-sizing:border-box}html{-webkit-text-size-adjust:100%}
body{margin:0;background:var(--bg);color:var(--ink);font:17px/1.6 var(--serif)}
a{color:var(--accent2);text-decoration-thickness:1px;text-underline-offset:2px}a:hover{color:var(--accent)}
:focus-visible{outline:2px solid var(--accent2);outline-offset:2px}
.vh{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0}
header.top{border-bottom:1px solid var(--rule);background:var(--paper)}
.wrap{max-width:1080px;margin:0 auto;padding:0 20px}
.top .wrap{display:flex;align-items:center;gap:4px 16px;min-height:58px;flex-wrap:wrap;padding-top:8px;padding-bottom:8px}
.brand{font:600 19px/1 var(--serif);color:var(--ink);text-decoration:none;letter-spacing:.01em;padding:4px 0}.brand span{color:var(--accent)}
nav.main{display:flex;gap:0 12px;font:14px var(--sans);flex-wrap:wrap}nav.main a{display:inline-block;padding:4px 0;line-height:18px;color:var(--ink2);text-decoration:none}nav.main a:hover,nav.main a.on{color:var(--ink)}nav.main a.on{text-decoration:underline;text-underline-offset:5px}
.spacer{flex:1}.acct{display:inline-flex;gap:10px;align-items:center;font:13px var(--sans)}.acct a{display:inline-block;padding:4px 0;color:var(--ink2)}.acct form{display:inline;margin:0}.acct button{background:none;border:0;padding:4px 0;font:13px var(--sans);color:var(--ink2);text-decoration:underline;text-underline-offset:2px;cursor:pointer}.acct button:hover{color:var(--accent)}.btn{display:inline-block;font:600 14px var(--sans);padding:8px 14px;border-radius:6px;border:1px solid var(--ink);background:var(--ink);color:var(--paper);text-decoration:none;cursor:pointer}
.btn.ghost{background:transparent;color:var(--ink)}.btn:hover{opacity:.88;color:var(--paper)}.btn.ghost:hover{color:var(--ink)}
main{padding:28px 0 60px}
.essay{max-width:720px;margin:0 auto}.wide{max-width:1080px;margin:0 auto}
h1{font:600 40px/1.15 var(--serif);margin:.2em 0 .3em;letter-spacing:-.01em;overflow-wrap:anywhere}h2{font:600 27px/1.25 var(--serif);margin:2.2em 0 .5em}h3,h2.h3{font:600 20px/1.3 var(--serif);margin:1.6em 0 .4em}
.kicker{font:600 12px var(--sans);letter-spacing:.12em;text-transform:uppercase;color:var(--accent)}
.lede{font-size:21px;line-height:1.5;color:var(--ink2)}
.byline{font:14px var(--sans);color:var(--ink3);margin:.6em 0 1.6em}
p{margin:.85em 0}
.figure{margin:26px -40px 30px;background:var(--paper);border:1px solid var(--rule);border-radius:10px;padding:18px 20px 14px}
.figure.inline{margin:22px 0}
.figure .ft,h2.minor{font:600 15px/1.35 var(--sans);margin:0 0 4px;letter-spacing:0}h2.minor{margin-bottom:8px}.figure .sub{font:13px var(--sans);color:var(--ink2);margin:0 0 10px}
.figure .src{font:12px var(--sans);color:var(--ink3);margin-top:8px}
.figure svg,.chart svg{width:100%;height:auto;display:block}
.chart .phone{display:none}
svg text{font-family:var(--sans)}
.stats{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:12px;margin:24px 0}
.stat{background:var(--paper);border:1px solid var(--rule);border-radius:8px;padding:12px 14px}.stat b{display:block;font:600 26px/1.1 var(--serif)}.stat span{display:block;font:13px/1.35 var(--sans);color:var(--ink2);margin-top:4px}
table.t{width:100%;border-collapse:collapse;font:14px/1.45 var(--sans);margin:12px 0}
table.t th{text-align:left;font-weight:600;color:var(--ink2);border-bottom:1px solid var(--ink3);padding:6px 8px;white-space:nowrap}
table.t td{border-bottom:1px solid var(--rule);padding:6px 8px;vertical-align:top}table.t td.n{text-align:right;font-variant-numeric:tabular-nums}
.heat{overflow-x:auto}.heat table.t{font-size:12.5px;margin:6px 0}
.heat th[scope=col]{text-align:center;font-weight:500;padding:6px 3px}
.heat th[scope=row]{position:sticky;left:0;z-index:1;background:var(--paper);font-weight:400;color:var(--ink);border-bottom:1px solid var(--rule);padding:5px 10px 5px 0}
.heat th[scope=row] a{display:inline-block;padding:2px 0}
.heat td{text-align:center;padding:5px 2px;color:var(--ink);font-variant-numeric:tabular-nums}
.swipe{display:none}
.hbars{font:13.5px/1.35 var(--sans)}
.hb{display:grid;grid-template-columns:minmax(120px,38%) 1fr 64px;gap:2px 10px;align-items:center;margin:4px 0}
.hb a{display:inline-block;padding:3px 0}
.hb .bar{background:var(--chip);border-radius:3px;height:14px}.hb .bar div{height:14px;border-radius:3px}
.hb .n{text-align:right;font-variant-numeric:tabular-nums;color:var(--ink2)}
.note{font:14px/1.5 var(--sans);color:var(--ink2);background:var(--chip);border-radius:8px;padding:12px 14px;margin:18px 0}
.chip{display:inline-block;font:12px/16px var(--sans);background:var(--chip);color:var(--ink2);border-radius:20px;padding:4px 10px;margin:0 4px 4px 0;text-decoration:none}
a.chip:hover{background:var(--rule);color:var(--ink)}
.chip.weak{background:transparent;border:1px solid var(--rule);color:var(--ink3);padding:3px 9px}
.chip.stop{background:transparent;color:var(--ink3);text-decoration:line-through}
.chip.topic{font-size:11px;line-height:14px;padding:5px 8px;margin-bottom:0;vertical-align:1px}
a.opening{font-style:italic}
p.titlenote{margin:-6px 0 12px;color:var(--ink3)}
p.snip{font-size:15px;line-height:1.55;margin:6px 0;overflow-wrap:anywhere}
.grid2{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:20px}.grid3{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:16px}
.card{background:var(--paper);border:1px solid var(--rule);border-radius:10px;padding:16px 18px;min-width:0;overflow-wrap:anywhere}.card h3,.card h2.h3{margin-top:0}
footer{border-top:1px solid var(--rule);font:13px/1.6 var(--sans);color:var(--ink3);padding:24px 0 40px}
.lock{font:13px var(--sans);color:var(--ink2)}.lock:before{content:"\1F512  "}
form.stack label{display:block;font:600 13px var(--sans);margin:14px 0 4px}
input[type=text],input[type=email],input[type=password],input[type=search],input[type=date],select,textarea{width:100%;font:16px var(--sans);padding:9px 11px;border:1px solid var(--grey);border-radius:6px;background:var(--paper);color:var(--ink)}
textarea{min-height:90px}
.searchbar{display:flex;gap:8px;flex-wrap:wrap}.searchbar input[type=search]{flex:1;min-width:min(220px,100%)}
.filters{display:flex;gap:8px;flex-wrap:wrap;margin:10px 0;font:13px var(--sans)}.filters select,.filters input{width:auto;max-width:100%;font-size:13px;padding:6px 8px}.t select{width:auto;font-size:13px;padding:5px 8px}
.results{list-style:none;padding:0;margin:16px 0}.results li{padding:12px 0;border-bottom:1px solid var(--rule)}
.results .meta{font:12.5px var(--sans);color:var(--ink3)}.results .title{font:600 17px/1.35 var(--serif);overflow-wrap:anywhere}.results .title a:not(.chip){display:inline-block;padding:2px 0}
.results .snip{font-size:15px;color:var(--ink2);margin-top:3px;overflow-wrap:anywhere}mark{background:var(--hl);color:inherit;padding:0 1px}
.reader{display:grid;grid-template-columns:minmax(0,1fr) 270px;gap:36px}
.msg{background:var(--paper);border:1px solid var(--rule);border-radius:10px;padding:30px 38px;min-width:0}
.msg h1{font-size:30px}.msg .body{overflow-wrap:anywhere}.msg .body p{margin:.9em 0;hyphens:auto}.msg .body blockquote{margin:1em 0;padding:.2em 1em;border-left:3px solid var(--rule);color:var(--ink2)}
.msg .body pre,.raw{font:13.5px/1.5 var(--mono);white-space:pre-wrap;overflow-wrap:anywhere}
.side{font:13.5px/1.5 var(--sans);overflow-wrap:anywhere;min-width:0}.side h2{font:600 12px var(--sans);letter-spacing:.08em;text-transform:uppercase;color:var(--ink3);margin:18px 0 6px}
.side dl{margin:0}.side dt{color:var(--ink3);font-size:12px}.side dd{margin:0 0 8px}
.cite{font:13px/1.5 var(--sans);background:var(--chip);border-radius:6px;padding:10px;user-select:all;overflow-wrap:anywhere}
details summary{cursor:pointer;font:600 13px var(--sans);color:var(--ink2);padding:4px 0}
.cal{display:grid;grid-template-columns:52px repeat(12,minmax(0,1fr));gap:3px;font:12px var(--sans)}
.cal a,.cal span{display:block;text-align:center;padding:6px 0;border-radius:4px;text-decoration:none;color:var(--ink)}
.cal .y{text-align:right;padding-right:8px;color:var(--ink2);font-weight:600}
.mcal{display:grid;grid-template-columns:repeat(7,minmax(0,1fr));gap:4px;font:13px/1.3 var(--sans);margin-top:12px}
.mcal .wd{text-align:center;padding:2px 0}
.mcal a,.mcal .none{display:block;min-height:58px;padding:6px 8px;border-radius:6px;text-decoration:none;color:var(--ink)}
.mcal b{display:block;font-weight:600}.mcal .n{display:block;font-size:12px}
.mcal .none{color:var(--ink3);border:1px dashed var(--rule)}.mcal .dash{display:none}
.pager{display:flex;flex-wrap:wrap;gap:4px 14px;font:14px var(--sans);margin:18px 0}.pager a{display:inline-block;padding:4px 0}
.flash{font:14px var(--sans);padding:10px 14px;border-radius:6px;background:#fdecea;color:#7a1c12;margin:10px 0}
.ok{background:#e8f4ea;color:#1f5b2a}
.timeline{border-left:2px solid var(--rule);margin:10px 0 10px 8px;padding-left:18px}.timeline .ev{margin:0 0 14px;position:relative}
.timeline .ev:before{content:"";position:absolute;left:-25px;top:8px;width:10px;height:10px;border-radius:50%;background:var(--accent)}
.timeline .d{font:600 12.5px var(--sans);color:var(--ink3)}
.terms{display:grid;grid-template-columns:56px minmax(0,1fr);gap:10px 14px;font:14px/1.5 var(--sans);align-items:baseline}.terms>div{display:flex;flex-wrap:wrap;gap:2px 0}.terms b{font-family:var(--serif)}
.small{font:13px var(--sans);color:var(--ink3)}
.langs{font:13px var(--sans);color:var(--ink3)}.langs a,.langs b{display:inline-block;min-width:24px;padding:4px 0;text-align:center}.langs a{color:var(--ink2);text-decoration:underline;text-underline-offset:3px}.langs b{color:var(--ink)}
.withheld{font-style:italic;color:var(--ink3)}
.sugg{font:12px var(--sans);border-radius:4px;padding:1px 6px}.sugg.restrict{background:#fdecea;color:#7a1c12}.sugg.show{background:#e8f4ea;color:#1f5b2a}
@media (max-width:860px){.figure{margin:22px 0}.reader{grid-template-columns:minmax(0,1fr)}.grid2,.grid3{grid-template-columns:minmax(0,1fr)}.msg{padding:20px}h1{font-size:31px}.lede{font-size:19px}}
@media (max-width:640px){
.wrap{padding:0 16px}main{padding:20px 0 48px}
h1{font-size:28px}.msg h1{font-size:24px}h2{font-size:23px;margin-top:1.8em}.lede{font-size:18px}
.figure{padding:14px 12px 10px;margin:20px 0}
.chart .desk{display:none}.chart .phone{display:block}
.swipe{display:block;font:12px var(--sans);color:var(--ink3);margin:4px 0 0}
table.t{display:block;overflow-x:auto;max-width:100%}.heat table.t{display:table;max-width:none}
.heat th[scope=row]{white-space:normal;min-width:118px;max-width:150px;font-size:12px;line-height:1.3;padding-right:8px}
.hb{grid-template-columns:minmax(0,1fr) 56px}.hb>div:first-child{grid-column:1/-1}
.msg{padding:16px 14px}
.cal{grid-template-columns:40px repeat(12,minmax(34px,1fr));overflow-x:auto;padding-bottom:6px}.cal .y,.cal>span:first-child{position:sticky;left:0;z-index:1;background:var(--bg)}
.mcal{gap:3px;font-size:12px}.mcal a,.mcal .none{min-height:46px;padding:4px 2px;text-align:center}.mcal .unit{display:none}.mcal .dash{display:inline}
.terms{grid-template-columns:44px minmax(0,1fr)}
}
@media print{header.top,footer,.side,.noprint{display:none}.msg{border:0;padding:0}body{background:#fff}.chart .phone{display:none}.chart .desk{display:block}}
"#;

/// The hidden form field that every form posting to the site carries (see security.rs).
pub fn csrf_field() -> Markup {
    let tok = i18n::ctx().map(|c| c.csrf.to_string()).unwrap_or_default();
    html! { input type="hidden" name="_csrf" value=(tok); }
}

/// The current page in the other language.
fn lang_link(code: &str) -> String {
    let path = i18n::ctx().map(|c| c.path).unwrap_or_else(|| "/".into());
    let sep = if path.contains('?') { '&' } else { '?' };
    format!("{path}{sep}lang={code}")
}

pub fn page(title: &str, user: Option<&User>, active: &str, body: Markup) -> Markup {
    let id = i18n::is_id();
    html! {
        (DOCTYPE)
        html lang=(if id { "id" } else { "en" }) {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " · " (t("Apakabar Archive", "Arsip Apakabar")) }
                meta name="description" content=(t("The apakabar lists of John A. MacDougall (1990–2002): analysis and a reading room of 136,000 recovered messages on Indonesia.", "Milis apakabar John A. MacDougall (1990–2002): analisis dan ruang baca berisi 136.000 pesan tentang Indonesia yang berhasil dipulihkan."));
                meta name="robots" content="noarchive, noai, noimageai";
                link rel="stylesheet" href=(format!("/style.css?v={}", css_version()));
                link rel="icon" href="data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><rect width='32' height='32' rx='6' fill='%239b2c1f'/><text x='16' y='22' font-size='17' text-anchor='middle' fill='white' font-family='Georgia'>ak</text></svg>";
            }
            body {
                header.top { div.wrap {
                    a.brand href="/" { @if id { "Arsip " span { "Apakabar" } } @else { "Apakabar " span { "Archive" } } }
                    nav.main {
                        a href="/" class=[(active == "home").then_some("on")] { (t("Analysis", "Analisis")) }
                        a href="/analysis/themes" class=[(active == "themes").then_some("on")] { (t("Themes", "Tema")) }
                        a href="/analysis/sources" class=[(active == "sources").then_some("on")] { (t("Sources", "Sumber")) }
                        a href="/analysis/events" class=[(active == "events").then_some("on")] { (t("Events", "Peristiwa")) }
                        a href="/analysis/people" class=[(active == "people").then_some("on")] { (t("People", "Tokoh")) }
                        a href="/analysis/words" class=[(active == "words").then_some("on")] { (t("Words", "Kata")) }
                        a href="/catalogue" class=[(active == "catalogue").then_some("on")] { (t("Catalogue", "Katalog")) }
                        a href="/about" class=[(active == "about").then_some("on")] { (t("About", "Tentang")) }
                    }
                    div.spacer {}
                    span.langs {
                        @if id { a href=(lang_link("en")) lang="en" title="English" { "EN" } " · " b title="Bahasa Indonesia" { "ID" } }
                        @else { b title="English" { "EN" } " · " a href=(lang_link("id")) lang="id" title="Bahasa Indonesia" { "ID" } }
                    }
                    @match user {
                        Some(u) => {
                            a.btn href="/read" { (t("Reading room", "Ruang baca")) }
                            span.acct {
                                a href="/account" title=(u.email) { (t("Account", "Akun")) }
                                form method="post" action="/logout" { (csrf_field()) button type="submit" title=(u.email) { (t("Log out", "Keluar")) } }
                            }
                        }
                        None => { a.btn href="/login" title=(t("Log in to read the sources", "Masuk untuk membaca sumber")) { (t("Log in", "Masuk")) } }
                    }
                }}
                main { div.wrap { (body) } }
                footer { div.wrap {
                    p { (t("Apakabar Archive. The lists were created and moderated by John A. MacDougall (Indonesia Publications), October 1990 to February 2002. Texts recovered from Internet Archive captures of the Hamline University and Ohio University Library databases; analysis and site by Nathanael Pribady, 2026.",
                        "Arsip Apakabar. Milis ini didirikan dan dimoderatori oleh John A. MacDougall (Indonesia Publications), Oktober 1990 sampai Februari 2002. Teks dipulihkan dari rekaman Internet Archive atas basis data Hamline University dan Ohio University Library; analisis dan situs oleh Nathanael Pribady, 2026.")) }
                    p { (t("Full texts are available to registered readers for research only. Most messages repost copyrighted articles; please cite and do not republish. ", "Teks lengkap hanya tersedia bagi pembaca terdaftar untuk keperluan penelitian. Sebagian besar pesan memuat ulang artikel yang dilindungi hak cipta; kutip sumbernya dan jangan menerbitkan ulang. "))
                        a href="/about#takedown" { (t("Corrections and takedown requests", "Koreksi dan permintaan penghapusan")) } "." }
                    @if !contacts().is_empty() { p { (t("Contact: ", "Kontak: ")) (contact_links()) "." } }
                }}
            }
        }
    }
}

pub fn esc_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}
