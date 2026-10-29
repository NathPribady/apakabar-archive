//! What every request passes through: the language and form token of the browser, the
//! check of every form post, the client address behind a proxy, limits on repeated login
//! attempts, and the security headers.
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
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const CSRF_COOKIE: &str = "ak_csrf";
pub const LANG_COOKIE: &str = "ak_lang";
const MAX_FORM: usize = 1 << 20;

/// The address a request came from: the first X-Forwarded-For entry behind a trusted proxy
/// (Replit's), otherwise the socket's peer.
#[derive(Clone, Debug)]
pub struct ClientIp(pub String);

#[axum::async_trait]
impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(parts.extensions.get::<ClientIp>().cloned().unwrap_or(ClientIp("unknown".into())))
    }
}

fn client_ip(req: &Request, trust_proxy: bool) -> String {
    if trust_proxy {
        if let Some(f) = req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if let Some(first) = f.split(',').next().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                return first.to_string();
            }
        }
    }
    req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip().to_string()).unwrap_or_else(|| "unknown".into())
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
    h.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'self'"),
    );
    if secure {
        h.insert("strict-transport-security", HeaderValue::from_static("max-age=31536000"));
    }
    if private {
        // full texts and account pages are never kept by shared caches
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    }
    if !h.contains_key(header::CONTENT_LANGUAGE) {
        h.insert(header::CONTENT_LANGUAGE, HeaderValue::from_static(if lang == Lang::Id { "id" } else { "en" }));
    }
    let _ = i18n::lang;
    res
}

// ---------------------------------------------------------------- repeated attempts

/// Counts recent events by key (a client address, an account) to slow down guessing.
#[derive(Default)]
pub struct Throttle(Mutex<HashMap<String, Vec<Instant>>>);

impl Throttle {
    /// Events under `key` in the last `window`.
    pub fn recent(&self, key: &str, window: Duration) -> usize {
        let m = self.0.lock().unwrap();
        m.get(key).map(|v| v.iter().filter(|t| t.elapsed() < window).count()).unwrap_or(0)
    }

    pub fn note(&self, key: &str) {
        let mut m = self.0.lock().unwrap();
        let v = m.entry(key.to_string()).or_default();
        v.retain(|t| t.elapsed() < Duration::from_secs(3600));
        v.push(Instant::now());
        if m.len() > 50_000 {
            m.retain(|_, v| v.last().map(|t| t.elapsed() < Duration::from_secs(900)).unwrap_or(false));
        }
    }

    pub fn clear(&self, key: &str) {
        self.0.lock().unwrap().remove(key);
    }
}

/// Limits: failed logins per address and per account in 15 minutes; registrations and
/// reset-form posts per address in an hour.
pub const LOGIN_WINDOW: Duration = Duration::from_secs(15 * 60);
pub const LOGIN_PER_IP: usize = 10;
pub const LOGIN_PER_ACCOUNT: usize = 5;
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
        th.clear("k");
        assert_eq!(th.recent("k", Duration::from_secs(60)), 0);
    }
}
