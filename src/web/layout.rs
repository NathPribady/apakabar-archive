//! Page shell and stylesheet. No JavaScript frameworks and no web fonts: pages are
//! server-rendered HTML with inline SVG charts, a few kilobytes each.

use super::auth::User;
use super::i18n::{self, t};
use maud::{html, Markup, DOCTYPE};

pub use super::i18n::{fmt_n, nice_date, pct};

pub const CSS: &str = r#"
:root{--bg:#fbfaf7;--paper:#ffffff;--ink:#1d1b18;--ink2:#5b5750;--ink3:#8a857c;--rule:#e4e0d8;--accent:#9b2c1f;--accent2:#1f5f8b;
--id:#9b2c1f;--en:#1f5f8b;--mixed:#c08a2b;--other:#b9b4aa;--hl:#fff1b8;--chip:#f1eee8;--teal:#2a8c82;--plum:#7d4e8c;
--serif:"Iowan Old Style","Palatino Linotype",Palatino,"Book Antiqua",Georgia,serif;--sans:system-ui,-apple-system,"Segoe UI",Roboto,"Helvetica Neue",Arial,sans-serif;--mono:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
@media (prefers-color-scheme:dark){:root:not([data-theme=light]){--bg:#15140f;--paper:#1c1b16;--ink:#ece8df;--ink2:#b5afa3;--ink3:#857f74;--rule:#34312a;--accent:#e07a5f;--accent2:#7fb3d5;--id:#e07a5f;--en:#7fb3d5;--mixed:#e0b65f;--other:#5d594f;--hl:#4a3f12;--chip:#26241e;--teal:#5cc4b8;--plum:#b98ccb}}
*{box-sizing:border-box}html{-webkit-text-size-adjust:100%}
body{margin:0;background:var(--bg);color:var(--ink);font:17px/1.6 var(--serif)}
a{color:var(--accent2);text-decoration-thickness:1px;text-underline-offset:2px}a:hover{color:var(--accent)}
header.top{border-bottom:1px solid var(--rule);background:var(--paper)}
.wrap{max-width:1080px;margin:0 auto;padding:0 20px}
.top .wrap{display:flex;align-items:center;gap:8px 16px;min-height:58px;flex-wrap:wrap;padding-top:8px;padding-bottom:8px}
.brand{font:600 19px/1 var(--serif);color:var(--ink);text-decoration:none;letter-spacing:.01em}.brand span{color:var(--accent)}
nav.main{display:flex;gap:6px 14px;font:14px var(--sans);flex-wrap:wrap}nav.main a{color:var(--ink2);text-decoration:none}nav.main a:hover,nav.main a.on{color:var(--ink)}
.spacer{flex:1}.acct{display:inline-flex;gap:10px;align-items:center;font:13px var(--sans)}.acct a{color:var(--ink2)}.acct form{display:inline;margin:0}.acct button{background:none;border:0;padding:0;font:13px var(--sans);color:var(--ink2);text-decoration:underline;text-underline-offset:2px;cursor:pointer}.acct button:hover{color:var(--accent)}.btn{display:inline-block;font:600 14px var(--sans);padding:8px 14px;border-radius:6px;border:1px solid var(--ink);background:var(--ink);color:var(--paper);text-decoration:none;cursor:pointer}
.btn.ghost{background:transparent;color:var(--ink)}.btn:hover{opacity:.88;color:var(--paper)}.btn.ghost:hover{color:var(--ink)}
main{padding:28px 0 60px}
.essay{max-width:720px;margin:0 auto}.wide{max-width:1080px;margin:0 auto}
h1{font:600 40px/1.15 var(--serif);margin:.2em 0 .3em;letter-spacing:-.01em}h2{font:600 27px/1.25 var(--serif);margin:2.2em 0 .5em}h3{font:600 20px/1.3 var(--serif);margin:1.6em 0 .4em}
.kicker{font:600 12px var(--sans);letter-spacing:.12em;text-transform:uppercase;color:var(--accent)}
.lede{font-size:21px;line-height:1.5;color:var(--ink2)}
.byline{font:14px var(--sans);color:var(--ink3);margin:.6em 0 1.6em}
p{margin:.85em 0}
.figure{margin:26px -40px 30px;background:var(--paper);border:1px solid var(--rule);border-radius:10px;padding:18px 20px 14px}
.figure.inline{margin:22px 0}
.figure h4{font:600 15px/1.35 var(--sans);margin:0 0 4px}.figure .sub{font:13px var(--sans);color:var(--ink2);margin:0 0 10px}
.figure .src{font:12px var(--sans);color:var(--ink3);margin-top:8px}
.figure svg{width:100%;height:auto;display:block}
svg text{font-family:var(--sans)}
.stats{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:12px;margin:24px 0}
.stat{background:var(--paper);border:1px solid var(--rule);border-radius:8px;padding:12px 14px}.stat b{display:block;font:600 26px/1.1 var(--serif)}.stat span{display:block;font:13px/1.35 var(--sans);color:var(--ink2);margin-top:4px}
table.t{width:100%;border-collapse:collapse;font:14px/1.45 var(--sans);margin:12px 0}
table.t th{text-align:left;font-weight:600;color:var(--ink2);border-bottom:1px solid var(--ink3);padding:6px 8px;white-space:nowrap}
table.t td{border-bottom:1px solid var(--rule);padding:6px 8px;vertical-align:top}table.t td.n{text-align:right;font-variant-numeric:tabular-nums}
.note{font:14px/1.5 var(--sans);color:var(--ink2);background:var(--chip);border-radius:8px;padding:12px 14px;margin:18px 0}
.chip{display:inline-block;font:12px var(--sans);background:var(--chip);color:var(--ink2);border-radius:20px;padding:2px 9px;margin:0 4px 4px 0;text-decoration:none}
a.chip:hover{background:var(--rule);color:var(--ink)}
.chip.weak{background:transparent;border:1px solid var(--rule);color:var(--ink3)}
.chip.stop{background:transparent;color:var(--ink3);text-decoration:line-through}
.chip.topic{font-size:11px;padding:1px 7px;vertical-align:2px}
a.opening{font-style:italic}
p.titlenote{margin:-6px 0 12px;color:var(--ink3)}
p.snip{font-size:15px;line-height:1.55;margin:6px 0}
.grid2{display:grid;grid-template-columns:1fr 1fr;gap:20px}.grid3{display:grid;grid-template-columns:repeat(3,1fr);gap:16px}
.card{background:var(--paper);border:1px solid var(--rule);border-radius:10px;padding:16px 18px}.card h3{margin-top:0}
footer{border-top:1px solid var(--rule);font:13px/1.6 var(--sans);color:var(--ink3);padding:24px 0 40px}
.lock{font:13px var(--sans);color:var(--ink2)}.lock:before{content:"\1F512  "}
form.stack label{display:block;font:600 13px var(--sans);margin:14px 0 4px}
input[type=text],input[type=email],input[type=password],input[type=search],input[type=date],select,textarea{width:100%;font:16px var(--sans);padding:9px 11px;border:1px solid var(--rule);border-radius:6px;background:var(--paper);color:var(--ink)}
textarea{min-height:90px}
.searchbar{display:flex;gap:8px;flex-wrap:wrap}.searchbar input[type=search]{flex:1;min-width:220px}
.filters{display:flex;gap:8px;flex-wrap:wrap;margin:10px 0;font:13px var(--sans)}.filters select,.filters input{width:auto;font-size:13px;padding:6px 8px}.t select{width:auto;font-size:13px;padding:5px 8px}
.results{list-style:none;padding:0;margin:16px 0}.results li{padding:12px 0;border-bottom:1px solid var(--rule)}
.results .meta{font:12.5px var(--sans);color:var(--ink3)}.results .title{font:600 17px/1.35 var(--serif)}
.results .snip{font-size:15px;color:var(--ink2);margin-top:3px}mark{background:var(--hl);color:inherit;padding:0 1px}
.reader{display:grid;grid-template-columns:minmax(0,1fr) 270px;gap:36px}
.msg{background:var(--paper);border:1px solid var(--rule);border-radius:10px;padding:30px 38px}
.msg h1{font-size:30px}.msg .body p{margin:.9em 0;hyphens:auto}.msg .body blockquote{margin:1em 0;padding:.2em 1em;border-left:3px solid var(--rule);color:var(--ink2)}
.msg .body pre,.raw{font:13.5px/1.5 var(--mono);white-space:pre-wrap;word-wrap:break-word}
.side{font:13.5px/1.5 var(--sans);overflow-wrap:anywhere}.side h4{font:600 12px var(--sans);letter-spacing:.08em;text-transform:uppercase;color:var(--ink3);margin:18px 0 6px}
.side dl{margin:0}.side dt{color:var(--ink3);font-size:12px}.side dd{margin:0 0 8px}
.cite{font:13px/1.5 var(--sans);background:var(--chip);border-radius:6px;padding:10px;user-select:all;overflow-wrap:anywhere}
details summary{cursor:pointer;font:600 13px var(--sans);color:var(--ink2)}
.cal{display:grid;grid-template-columns:52px repeat(12,1fr);gap:3px;font:12px var(--sans)}
.cal a,.cal span{display:block;text-align:center;padding:6px 0;border-radius:4px;text-decoration:none;color:var(--ink)}
.cal .y{text-align:right;padding-right:8px;color:var(--ink2);font-weight:600}
.pager{display:flex;gap:10px;font:14px var(--sans);margin:18px 0}
.flash{font:14px var(--sans);padding:10px 14px;border-radius:6px;background:#fdecea;color:#7a1c12;margin:10px 0}
.ok{background:#e8f4ea;color:#1f5b2a}
.timeline{border-left:2px solid var(--rule);margin:10px 0 10px 8px;padding-left:18px}.timeline .ev{margin:0 0 14px;position:relative}
.timeline .ev:before{content:"";position:absolute;left:-25px;top:8px;width:10px;height:10px;border-radius:50%;background:var(--accent)}
.timeline .d{font:600 12.5px var(--sans);color:var(--ink3)}
.terms{display:grid;grid-template-columns:56px minmax(0,1fr);gap:10px 14px;font:14px/1.5 var(--sans);align-items:baseline}.terms>div{display:flex;flex-wrap:wrap;gap:2px 0}.terms b{font-family:var(--serif)}
.small{font:13px var(--sans);color:var(--ink3)}
.langs{font:13px var(--sans);color:var(--ink3)}.langs a{color:var(--ink2);text-decoration:none}.langs b{color:var(--ink)}
.withheld{font-style:italic;color:var(--ink3)}
.sugg{font:12px var(--sans);border-radius:4px;padding:1px 6px}.sugg.restrict{background:#fdecea;color:#7a1c12}.sugg.show{background:#e8f4ea;color:#1f5b2a}
@media (max-width:860px){.figure{margin:22px 0}.reader{grid-template-columns:1fr}.grid2,.grid3{grid-template-columns:1fr}.msg{padding:20px}h1{font-size:31px}.lede{font-size:19px}.cal{grid-template-columns:40px repeat(12,1fr);font-size:10.5px}}
@media print{header.top,footer,.side,.noprint{display:none}.msg{border:0;padding:0}body{background:#fff}}
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
                link rel="stylesheet" href="/style.css";
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
                }}
            }
        }
    }
}

pub fn esc_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}
