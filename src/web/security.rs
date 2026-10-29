//! What every request passes through: the refusal of AI crawlers, scrapers and other
//! automated clients, the pace limits per client address, the language and form token of the
//! browser, the check of every form post, the client address behind a proxy, limits on
//! repeated login attempts, and the security headers.
//!
//! Form posts carry a token (`_csrf`) that must match the browser's `ak_csrf` cookie
//! (double-submit). A site that tricks a reader's browser into posting a form cannot read
//! the cookie, so it cannot send the token; SameSite cookies and an Origin check add to this.

use super::i18n::{self, Ctx, Lang, CTX};
use super::AppState;
use axum::{
    body::Body,
    extract::{ConnectInfo, FromRequestParts, Request, State},
    http::{header, request::Parts, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const CSRF_COOKIE: &str = "ak_csrf";
pub const LANG_COOKIE: &str = "ak_lang";
const MAX_FORM: usize = 1 << 20;

/// The address a request came from (see `client_ip`), as the limits count it: an IPv4
/// address, or the /64 network of an IPv6 one (a single connection usually holds a whole /64,
/// so its addresses are one client).
#[derive(Clone, Debug)]
pub struct ClientIp(pub String);

fn limit_key(ip: IpAddr) -> String {
    match ip {
        IpAddr::V6(v) if v.to_ipv4_mapped().is_none() => {
            let g = v.segments();
            format!("{:x}:{:x}:{:x}:{:x}::/64", g[0], g[1], g[2], g[3])
        }
        IpAddr::V6(v) => v.to_ipv4_mapped().map(|m| m.to_string()).unwrap_or_default(),
        IpAddr::V4(v) => v.to_string(),
    }
}

#[axum::async_trait]
impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(parts.extensions.get::<ClientIp>().cloned().unwrap_or(ClientIp("unknown".into())))
    }
}

/// Addresses of the host's own network: private and shared ranges, loopback, link-local, and
/// the ranges Google's load balancers (in front of Replit) forward from.
fn infrastructure(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            v.is_private() || v.is_loopback() || v.is_link_local() || v.is_unspecified()
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10, shared address space
                || (o[0] == 35 && o[1] == 191) // 35.191.0.0/16, Google load balancers
                || (o[0] == 130 && o[1] == 211 && o[2] < 4) // 130.211.0.0/22, Google load balancers
        }
        IpAddr::V6(v) => {
            let s0 = v.segments()[0];
            v.is_loopback() || v.is_unspecified() || (s0 & 0xfe00) == 0xfc00 || (s0 & 0xffc0) == 0xfe80 || v.to_ipv4_mapped().map(|m| infrastructure(&IpAddr::V4(m))).unwrap_or(false)
        }
    }
}

/// The client's address. Behind a proxy (`--trust-proxy`), X-Forwarded-For lists the addresses
/// the request came through, and each proxy adds the one it saw at the end. A client can write
/// anything at the start, so the list is read from the end: the first address that is not the
/// host's own network is the client. Without a proxy, the socket's peer.
fn client_ip(req: &Request, trust_proxy: bool) -> String {
    if trust_proxy {
        if let Some(f) = req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if let Some(ip) = forwarded_client(f) {
                return limit_key(ip);
            }
        }
    }
    req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| limit_key(c.0.ip())).unwrap_or_else(|| "unknown".into())
}

fn forwarded_client(header: &str) -> Option<IpAddr> {
    let addrs: Vec<IpAddr> = header.split(',').filter_map(|p| p.trim().parse::<IpAddr>().ok()).collect();
    addrs.iter().rev().find(|ip| !infrastructure(ip)).or(addrs.first()).copied()
}

/// For the first few requests after a start, what the proxy's X-Forwarded-For looks like
/// (kinds of address only, never the addresses), so the reading above can be checked.
fn log_forwarding(header: &str) {
    static SEEN: AtomicUsize = AtomicUsize::new(0);
    if SEEN.fetch_add(1, Ordering::Relaxed) >= 3 {
        return;
    }
    let kinds: Vec<&str> = header.split(',').map(|p| match p.trim().parse::<IpAddr>() {
        Ok(ip) if infrastructure(&ip) => "proxy/private",
        Ok(_) => "public",
        Err(_) => "unreadable",
    }).collect();
    let pos = forwarded_client(header).and_then(|c| header.split(',').position(|p| p.trim().parse::<IpAddr>().ok() == Some(c))).map(|p| p + 1).unwrap_or(0);
    eprintln!("proxy check: X-Forwarded-For lists [{}]; the client is read from entry {pos}", kinds.join(", "));
}

fn loopback_peer(req: &Request) -> bool {
    req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip().is_loopback()).unwrap_or(false)
}

// ---------------------------------------------------------------- crawlers and scrapers

/// The search engines the editors let list the public analysis pages. robots.txt keeps them
/// out of the reading room, the accounts and the editors' pages.
pub const SEARCH_ENGINES: &[&str] = &["Googlebot", "Bingbot", "DuckDuckBot", "Applebot"];

/// AI crawlers and agents, archive and SEO crawlers and scraping services: named in
/// robots.txt, and refused by the server whatever they read there. Google-Extended and
/// Applebot-Extended are not crawlers: they are the tokens by which Google and Apple let a site
/// keep what their search crawlers read out of AI training and answers.
pub const REFUSED_AGENTS: &[&str] = &[
    "GPTBot", "OAI-SearchBot", "ChatGPT-User", "ChatGPT Agent", "OAI-AdsBot", "ClaudeBot", "Claude-User", "Claude-SearchBot", "Claude-Web", "anthropic-ai",
    "PerplexityBot", "Perplexity-User", "Google-Extended", "GoogleOther", "GoogleOther-Image", "GoogleOther-Video", "Google-CloudVertexBot", "Gemini-Deep-Research",
    "Applebot-Extended", "meta-externalagent", "meta-externalfetcher", "meta-webindexer", "FacebookBot", "Amazonbot", "Amzn-SearchBot", "Amzn-User",
    "MistralAI-User", "MistralAI-Index", "MistralAI-Training", "DuckAssistBot", "Bytespider", "TikTokSpider", "CCBot", "cohere-ai", "cohere-training-data-crawler",
    "AI2Bot", "Ai2Bot-Dolma", "Diffbot", "YouBot", "Timpibot", "ImagesiftBot", "Omgilibot", "Omgili", "img2dataset", "PanguBot", "Kangaroo Bot", "Webzio-Extended",
    "PetalBot", "GrokBot", "xAI-Bot", "iaskspider", "ISSCyberRiskCrawler", "Sidetrade indexer bot", "QualifiedBot", "Brightbot", "VelenPublicWebCrawler",
    "Crawlspace", "FriendlyCrawler", "Factset_spyderbot", "magpie-crawler", "NovaAct", "FirecrawlAgent", "Crawl4AI", "Scrapy", "ia_archiver", "archive.org_bot",
    "AhrefsBot", "SemrushBot", "MJ12bot", "DotBot", "BLEXBot", "DataForSeoBot",
];

/// Programs scrapers are written with, refused by the server (they say nothing in robots.txt).
const SCRAPER_TOOLS: &[&str] = &[
    "python-", "python/", "aiohttp", "httpx", "go-http-client", "java/", "okhttp", "axios", "node-fetch", "undici", "libwww-perl", "lwp::", "curl/", "wget",
    "headlesschrome", "phantomjs", "puppeteer", "playwright", "selenium", "colly", "apache-httpclient", "ruby", "mechanize", "httrack", "webcopier",
    "offline explorer", "sitesucker", "teleport pro", "webzip", "httpie", "guzzlehttp", "winhttp", "gemini", "jina", "scrapingbee", "zyte", "apify", "firecrawl",
];

/// Link previews of chat and social apps (one public page, when someone shares a link).
const LINK_PREVIEWS: &[&str] = &["facebookexternalhit", "twitterbot", "linkedinbot", "slackbot", "discordbot", "telegrambot", "whatsapp", "skypeuripreview"];

/// Whether the server refuses a client by what it says it is: every AI crawler and agent and
/// scraping tool, and anything else that calls itself a crawler or bot (or gives no name at
/// all), except the search engines above and link previews.
pub fn refused_agent(ua: &str) -> bool {
    let ua = ua.trim().to_ascii_lowercase();
    if ua.is_empty() || ua == "-" || ua == "mozilla/5.0" {
        return true;
    }
    if REFUSED_AGENTS.iter().any(|a| ua.contains(&a.to_ascii_lowercase())) || SCRAPER_TOOLS.iter().any(|a| ua.contains(a)) {
        return true;
    }
    let allowed = SEARCH_ENGINES.iter().any(|a| ua.contains(&a.to_ascii_lowercase())) || LINK_PREVIEWS.iter().any(|a| ua.contains(a)) || ua.contains("google-inspectiontool");
    let automated = ["http://", "https://", "+http", "bot/", "crawler", "spider", "scraper", "headless"].iter().any(|k| ua.contains(k));
    automated && !allowed
}

/// robots.txt: search engines may read the public pages; nothing else may read anything.
pub fn robots_txt() -> String {
    let mut s = String::from("# Apakabar Archive. Search engines may list the public analysis pages. The reading room,\n# accounts and editors' pages are closed to every crawler. AI crawlers and agents, archive and\n# SEO crawlers and scrapers may not use any part of the site, for training, answers or anything\n# else; the server refuses them. Text and data mining reserved (TDMRep: /.well-known/tdmrep.json).\n\n");
    for a in SEARCH_ENGINES {
        s.push_str(&format!("User-agent: {a}\n"));
    }
    for p in ["/read", "/login", "/register", "/account", "/reset", "/setup", "/admin", "/logout", "/catalogue?q="] {
        s.push_str(&format!("Disallow: {p}\n"));
    }
    s.push_str("Allow: /\n\n");
    for a in REFUSED_AGENTS {
        s.push_str(&format!("User-agent: {a}\n"));
    }
    s.push_str("Disallow: /\n\nUser-agent: *\nDisallow: /\n");
    s
}

// ---------------------------------------------------------------- pace limits per address

/// Requests per client address: 300 in 5 minutes (a reader turns a page every few seconds at
/// most; a scraper wants thousands), and 40 searches in 5 minutes.
pub const PACE_WINDOW: Duration = Duration::from_secs(300);
pub const PAGES_PER_ADDRESS: usize = 300;
pub const SEARCHES_PER_ADDRESS: usize = 40;

fn is_search(path: &str, query: Option<&str>) -> bool {
    // the keys decoded as the page reads them ("%71=x" is "q=x")
    let has_q = |q: &str| {
        q.split('&').any(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            let k = urlencoding::decode(&k.replace('+', " ")).map(|c| c.into_owned()).unwrap_or_default();
            k == "q" && !v.is_empty()
        })
    };
    path == "/read/search" || (path == "/catalogue" && query.map(has_q).unwrap_or(false))
}

fn too_many(lang: Lang) -> Response {
    let m = if lang == Lang::Id { "Terlalu banyak permintaan dari alamat Anda. Silakan coba lagi beberapa menit lagi." } else { "Too many requests from your address. Please try again in a few minutes." };
    (StatusCode::TOO_MANY_REQUESTS, [(header::CONTENT_TYPE, "text/plain; charset=utf-8"), (header::RETRY_AFTER, "300")], m).into_response()
}

fn refused_bot() -> Response {
    (
        StatusCode::FORBIDDEN,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8"), (header::HeaderName::from_static("x-robots-tag"), "noindex, nofollow")],
        "Automated access to the Apakabar Archive is not permitted: no AI crawlers or agents, scrapers or bulk downloads. See /robots.txt.\n",
    )
        .into_response()
}

/// Where the language switch leads on a page answered to a form: the same address when it
/// also has a page of its own, otherwise the section it belongs to.
fn after_post(path: &str) -> String {
    let own_page = ["/login", "/register", "/account", "/admin/review"].contains(&path) || path.starts_with("/reset/") || path.starts_with("/admin/checks/");
    if own_page {
        path.to_string()
    } else if path.starts_with("/admin") {
        "/admin".into()
    } else {
        "/".into()
    }
}

/// The path and query without `lang=`, so the language switch can link back to the page.
fn path_without_lang(uri: &axum::http::Uri) -> String {
    let q: Vec<&str> = uri.query().unwrap_or("").split('&').filter(|kv| !kv.is_empty() && !kv.starts_with("lang=")).collect();
    if q.is_empty() { uri.path().to_string() } else { format!("{}?{}", uri.path(), q.join("&")) }
}

fn valid_token(t: &str) -> bool {
    t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Compare without stopping at the first difference.
pub fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn form_field(body: &[u8], name: &str) -> Option<String> {
    let s = std::str::from_utf8(body).ok()?;
    s.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        if k == name { urlencoding::decode(&v.replace('+', " ")).ok().map(|x| x.into_owned()) } else { None }
    })
}

fn cookie(name: &str, value: &str, max_age: i64, secure: bool) -> HeaderValue {
    let mut c = format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}");
    if secure {
        c.push_str("; Secure");
    }
    HeaderValue::from_str(&c).unwrap_or_else(|_| HeaderValue::from_static(""))
}

fn refused(lang: Lang, msg_en: &str, msg_id: &str) -> Response {
    let m = if lang == Lang::Id { msg_id } else { msg_en };
    (StatusCode::FORBIDDEN, [(header::CONTENT_TYPE, "text/plain; charset=utf-8")], m.to_string()).into_response()
}

/// Sets up the request context (language, form token, client address), refuses form posts
/// without a valid token, and adds the security headers to every answer.
pub async fn context(State(st): State<AppState>, req: Request, next: Next) -> Response {
    // AI crawlers, scrapers and too-fast clients are turned away before anything else; the
    // host's own health check (from this machine, not through the proxy) and robots.txt are not
    let forwarded = req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok()).map(|s| s.to_string());
    if let Some(f) = &forwarded {
        if st.trust_proxy {
            log_forwarding(f);
        }
    }
    let internal = forwarded.is_none() && loopback_peer(&req);
    let open = matches!(req.uri().path(), "/robots.txt" | "/.well-known/tdmrep.json");
    if !internal && !open {
        let ua = req.headers().get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("");
        if refused_agent(ua) {
            return refused_bot();
        }
        let ip = client_ip(&req, st.trust_proxy);
        let lang = Lang::from_accept(req.headers().get(header::ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok()).unwrap_or(""));
        if req.uri().path() != "/style.css" {
            let key = format!("pace:{ip}");
            if st.throttle.recent(&key, PACE_WINDOW) >= PAGES_PER_ADDRESS {
                return too_many(lang);
            }
            st.throttle.note(&key);
        }
        if is_search(req.uri().path(), req.uri().query()) {
            let key = format!("search:{ip}");
            if st.throttle.recent(&key, PACE_WINDOW) >= SEARCHES_PER_ADDRESS {
                return too_many(lang);
            }
            st.throttle.note(&key);
        }
    }
    let jar = CookieJar::from_headers(req.headers());
    let q_lang = req.uri().query().and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("lang="))).and_then(Lang::parse);
    let lang = q_lang
        .or_else(|| jar.get(LANG_COOKIE).and_then(|c| Lang::parse(c.value())))
        .unwrap_or_else(|| Lang::from_accept(req.headers().get(header::ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok()).unwrap_or("")));
    let old_token = jar.get(CSRF_COOKIE).map(|c| c.value().to_string()).filter(|t| valid_token(t));
    let token = old_token.clone().unwrap_or_else(crate::site::new_token);
    let path = if req.method() == Method::GET { path_without_lang(req.uri()) } else { after_post(req.uri().path()) };
    let ip = client_ip(&req, st.trust_proxy);
    let secure = st.secure_cookies;
    let private = ["/read", "/admin", "/account", "/reset", "/setup"].iter().any(|p| req.uri().path().starts_with(p));

    let mut req = req;
    if req.method() == Method::POST {
        // belt and braces: a post from another site is refused before anything else
        let mut hosts: Vec<String> = Vec::new();
        if let Some(h) = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()) {
            hosts.push(h.to_string());
        }
        if st.trust_proxy {
            if let Some(h) = req.headers().get("x-forwarded-host").and_then(|v| v.to_str().ok()) {
                hosts.push(h.to_string());
            }
        }
        if let Some(b) = &st.base_host {
            hosts.push(b.clone());
        }
        if let Some(origin) = req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
            let o = origin.trim_start_matches("https://").trim_start_matches("http://");
            if origin != "null" && !hosts.iter().any(|h| h == o) {
                return refused(lang, "Cross-site request refused.", "Permintaan dari situs lain ditolak.");
            }
        }
        let (parts, body) = req.into_parts();
        let bytes = match axum::body::to_bytes(body, MAX_FORM).await {
            Ok(b) => b,
            Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "Form too large").into_response(),
        };
        let sent = form_field(&bytes, "_csrf").unwrap_or_default();
        match &old_token {
            Some(t) if same(t, &sent) => {}
            _ => {
                return refused(
                    lang,
                    "This form has expired. Go back, reload the page and send it again.",
                    "Formulir ini sudah kedaluwarsa. Kembali, muat ulang halaman, lalu kirim lagi.",
                )
            }
        }
        req = Request::from_parts(parts, Body::from(bytes));
    }
    req.extensions_mut().insert(ClientIp(ip));

    let ctx = Ctx { lang, path, csrf: Arc::new(token.clone()) };
    let mut res = CTX.scope(ctx, next.run(req)).await;
    let h = res.headers_mut();
    if old_token.is_none() {
        h.append(header::SET_COOKIE, cookie(CSRF_COOKIE, &token, 60 * 60 * 24 * 365, secure));
    }
    if let Some(l) = q_lang {
        h.append(header::SET_COOKIE, cookie(LANG_COOKIE, l.code(), 60 * 60 * 24 * 365, secure));
    }
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("referrer-policy", HeaderValue::from_static("same-origin"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("permissions-policy", HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=(), usb=(), interest-cohort=(), browsing-topics=()"));
    h.insert("cross-origin-opener-policy", HeaderValue::from_static("same-origin"));
    h.insert("cross-origin-resource-policy", HeaderValue::from_static("same-origin"));
    // no AI use of any page (noai/noimageai; noarchive also keeps Bing's AI answers out),
    // and text and data mining reserved (TDMRep)
    h.insert(
        "x-robots-tag",
        HeaderValue::from_static(if private { "noindex, nofollow, noarchive, noai, noimageai" } else { "noarchive, noai, noimageai" }),
    );
    h.insert("tdm-reservation", HeaderValue::from_static("1"));
    h.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'self'; script-src 'none'; object-src 'none'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; connect-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'"),
    );
    if secure {
        h.insert("strict-transport-security", HeaderValue::from_static("max-age=31536000"));
    }
    if private {
        // full texts and account pages are never kept by shared caches
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    } else if !h.contains_key(header::CACHE_CONTROL) {
        // pages carry the reader's form token (and, when logged in, their address): kept by
        // their own browser only
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    }
    if !h.contains_key(header::CONTENT_LANGUAGE) {
        h.insert(header::CONTENT_LANGUAGE, HeaderValue::from_static(if lang == Lang::Id { "id" } else { "en" }));
    }
    let _ = i18n::lang;
    res
}

// ---------------------------------------------------------------- repeated attempts

/// Counts recent events by key (a client address, an account) to slow down guessing and
/// scraping. Events older than an hour (the longest window) are dropped, at most once a minute.
pub struct Throttle(Mutex<(HashMap<String, Vec<Instant>>, Instant)>);

impl Default for Throttle {
    fn default() -> Self {
        Throttle(Mutex::new((HashMap::new(), Instant::now())))
    }
}

const KEEP: Duration = Duration::from_secs(3600);

impl Throttle {
    fn map(&self) -> std::sync::MutexGuard<'_, (HashMap<String, Vec<Instant>>, Instant)> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Events under `key` in the last `window`.
    pub fn recent(&self, key: &str, window: Duration) -> usize {
        self.map().0.get(key).map(|v| v.iter().filter(|t| t.elapsed() < window).count()).unwrap_or(0)
    }

    pub fn note(&self, key: &str) {
        let mut g = self.map();
        let (m, pruned) = &mut *g;
        if pruned.elapsed() > Duration::from_secs(60) {
            m.retain(|_, v| v.last().map(|t| t.elapsed() < KEEP).unwrap_or(false));
            *pruned = Instant::now();
        }
        let v = m.entry(key.to_string()).or_default();
        if v.len() >= 1000 {
            v.retain(|t| t.elapsed() < KEEP);
            if v.len() >= 1000 {
                v.remove(0);
            }
        }
        v.push(Instant::now());
    }

    /// Take back the latest event under `key` (an attempt that turned out right).
    pub fn forget_last(&self, key: &str) {
        if let Some(v) = self.map().0.get_mut(key) {
            v.pop();
        }
    }

    pub fn clear(&self, key: &str) {
        self.map().0.remove(key);
    }
}

/// Limits: failed logins per address and per account in 15 minutes; registrations and
/// reset-form posts per address in an hour.
pub const LOGIN_WINDOW: Duration = Duration::from_secs(15 * 60);
pub const LOGIN_PER_IP: usize = 10;
pub const LOGIN_PER_ACCOUNT: usize = 5;
pub const LOGIN_PER_ACCOUNT_ALL: usize = 30;
pub const REGISTER_WINDOW: Duration = Duration::from_secs(3600);
pub const REGISTER_PER_IP: usize = 5;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helpers() {
        assert!(same("abc", "abc") && !same("abc", "abd") && !same("abc", "ab"));
        assert_eq!(form_field(b"a=1&_csrf=x%2By&b=2", "_csrf").as_deref(), Some("x+y"));
        let u: axum::http::Uri = "/read/search?q=x&lang=id&page=2".parse().unwrap();
        assert_eq!(path_without_lang(&u), "/read/search?q=x&page=2");
        let th = Throttle::default();
        th.note("k");
        th.note("k");
        assert_eq!(th.recent("k", Duration::from_secs(60)), 2);
        th.forget_last("k");
        assert_eq!(th.recent("k", Duration::from_secs(60)), 1);
        th.clear("k");
        assert_eq!(th.recent("k", Duration::from_secs(60)), 0);
        assert!(is_search("/catalogue", Some("%71=x")) && is_search("/catalogue", Some("ym=1998-05&q=riot")));
        assert!(!is_search("/catalogue", Some("q=")) && !is_search("/catalogue", Some("ym=1998-05")));
    }

    #[test]
    fn who_is_refused() {
        let chrome = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
        let iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.6 Mobile/15E148 Safari/604.1";
        for ua in [chrome, iphone, "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)", "Mozilla/5.0 (compatible; bingbot/2.0; +http://www.bing.com/bingbot.htm)", "DuckDuckBot/1.1; (+http://duckduckgo.com/duckduckbot.html)", "facebookexternalhit/1.1"] {
            assert!(!refused_agent(ua), "{ua}");
        }
        for ua in [
            "Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; GPTBot/1.2; +https://openai.com/gptbot)",
            "Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; ClaudeBot/1.0; +claudebot@anthropic.com)",
            "Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; PerplexityBot/1.0; +https://perplexity.ai/perplexitybot)",
            "CCBot/2.0 (https://commoncrawl.org/faq/)",
            "Mozilla/5.0 (compatible; DuckAssistBot/1.2; +http://duckduckgo.com/duckassistbot.html)",
            "python-requests/2.32.3", "curl/8.5.0", "Wget/1.21", "Scrapy/2.11 (+https://scrapy.org)",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/120.0 Safari/537.36",
            "Mozilla/5.0 (compatible; SomeNewCrawler/1.0; +https://example.com/bot)",
            "", "-", "Mozilla/5.0",
        ] {
            assert!(refused_agent(ua), "{ua}");
        }
        let r = robots_txt();
        assert!(r.contains("User-agent: GPTBot") && r.ends_with("User-agent: *\nDisallow: /\n"));
    }

    #[test]
    fn addresses() {
        // read from the right, past the host's own proxies
        assert_eq!(forwarded_client("1.2.3.4, 203.0.113.9, 35.191.2.3").map(|i| i.to_string()).as_deref(), Some("203.0.113.9"));
        assert_eq!(forwarded_client("10.0.0.1").map(|i| i.to_string()).as_deref(), Some("10.0.0.1"));
        assert_eq!(limit_key("2001:db8:aa:bb:1:2:3:4".parse().unwrap()), "2001:db8:aa:bb::/64");
        assert_eq!(limit_key("::ffff:198.51.100.7".parse().unwrap()), "198.51.100.7");
    }
}
