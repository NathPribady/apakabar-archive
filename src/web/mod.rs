//! The website: a public analysis (essay and data pages) and a reading room behind a login.

pub mod account;
pub mod admin;
pub mod auth;
pub mod charts;
pub mod i18n;
pub mod layout;
pub mod public;
pub mod read;
pub mod review;
pub mod security;

use crate::site::{self, SiteConn, SiteDb};
use crate::{db, ingest::anyhow_lite::Result};
use axum::{
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde_json::{json, Value};
use once_cell::sync::Lazy;
use rusqlite::OptionalExtension;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// Messages ordinary readers may not open: the May 1998 messages no editor has reviewed yet,
/// and those the editors restricted. Researchers and editors see everything.
#[derive(Default, Debug)]
pub struct Restricted {
    /// May 1998 messages no editor has decided on yet
    pub pending: HashSet<String>,
    /// messages an editor marked sensitive
    pub sensitive: HashSet<String>,
    /// both, with every copy of each text: what readers may not open
    pub all: HashSet<String>,
}
impl Restricted {
    pub fn hidden(&self, ak: &str) -> bool {
        self.all.contains(ak)
    }
}

#[derive(Clone)]
pub struct AppState {
    /// the archive database (apakabar.db)
    pub pool: db::DbPool,
    /// accounts and editorial data (site.db or PostgreSQL)
    pub site: SiteDb,
    pub analysis: Arc<HashMap<String, Value>>,
    /// the precision checks, recounted from the site store at start and after each review
    pub checks: Arc<RwLock<Value>>,
    pub restricted: Arc<RwLock<Restricted>>,
    pub approval: bool,
    pub secure_cookies: bool,
    pub trust_proxy: bool,
    pub base_url: Option<String>,
    pub base_host: Option<String>,
    pub throttle: Arc<security::Throttle>,
    /// a one-time link to create the first editor account, while there is none
    pub setup: Arc<Mutex<Option<String>>>,
    /// the folder of the editable texts (content/home.en.md …)
    pub content: PathBuf,
}

impl AppState {
    pub fn a(&self, key: &str) -> &Value {
        static NULL: Value = Value::Null;
        self.analysis.get(key).unwrap_or(&NULL)
    }
    pub fn checks(&self) -> Value {
        self.checks.read().unwrap().clone()
    }
    pub fn hidden(&self, ak: &str) -> bool {
        self.restricted.read().unwrap().hidden(ak)
    }
    /// An absolute link, when the site's address is known (for links shown to editors).
    pub fn url(&self, path: &str) -> String {
        match &self.base_url {
            Some(b) => format!("{}{}", b.trim_end_matches('/'), path),
            None => path.to_string(),
        }
    }
}

/// Archive queries still running after this long are stopped: no reader should wait for one,
/// and a query built to keep the server busy must not.
pub const QUERY_LIMIT: Duration = Duration::from_secs(20);

/// The queries running, with their deadlines; one thread stops those past it.
static RUNNING: Lazy<Mutex<(u64, HashMap<u64, (Instant, rusqlite::InterruptHandle)>)>> = Lazy::new(|| {
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_millis(250));
        let now = Instant::now();
        RUNNING.lock().unwrap_or_else(|e| e.into_inner()).1.retain(|_, (deadline, h)| {
            if now >= *deadline {
                h.interrupt();
                false
            } else {
                true
            }
        });
    });
    Mutex::new((0, HashMap::new()))
});

/// A query under watch; dropped (also when the work panics) before its connection goes back
/// to the pool, so the watchdog never stops a later query on that connection.
struct Watched(u64);

impl Drop for Watched {
    fn drop(&mut self) {
        RUNNING.lock().unwrap_or_else(|e| e.into_inner()).1.remove(&self.0);
    }
}

/// Run blocking work on the archive database off the async threads, within QUERY_LIMIT.
pub async fn blocking<T: Send + 'static>(state: &AppState, f: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static) -> std::result::Result<T, AppError> {
    let pool = state.pool.clone();
    tokio::task::spawn_blocking(move || {
        let c = pool.get().map_err(|e| AppError(e.to_string()))?;
        let _watch = {
            let mut r = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
            r.0 += 1;
            let id = r.0;
            r.1.insert(id, (Instant::now() + QUERY_LIMIT, c.get_interrupt_handle()));
            Watched(id)
        };
        f(&c).map_err(|e| AppError(e.to_string()))
    })
    .await
    .map_err(|e| AppError(e.to_string()))?
}

/// Full-text searches run three at a time (readers' and the catalogue's together), so they
/// never hold every connection to the archive: pages keep loading while searches wait.
static SEARCHES: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(3);

pub async fn search_turn() -> Option<tokio::sync::SemaphorePermit<'static>> {
    SEARCHES.acquire().await.ok()
}

/// Run blocking work on the site store off the async threads.
pub async fn site<T: Send + 'static>(state: &AppState, f: impl FnOnce(&mut SiteConn) -> site::Res<T> + Send + 'static) -> std::result::Result<T, AppError> {
    let db = state.site.clone();
    tokio::task::spawn_blocking(move || {
        let mut c = db.get().map_err(|e| AppError(e.to_string()))?;
        f(&mut c).map_err(|e| AppError(e.to_string()))
    })
    .await
    .map_err(|e| AppError(e.to_string()))?
}

#[derive(Debug)]
pub struct AppError(pub String);
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        if self.0 == "bad date" || self.0 == "bad month" {
            // an address that names no real day or month
            return (StatusCode::NOT_FOUND, i18n::t("No such date.", "Tanggal itu tidak ada.").to_string()).into_response();
        }
        if self.0.contains("interrupted") {
            // stopped by QUERY_LIMIT
            return (StatusCode::SERVICE_UNAVAILABLE, i18n::t("That took too long. Please try again, or narrow the search.", "Terlalu lama. Silakan coba lagi, atau persempit pencarian.").to_string()).into_response();
        }
        eprintln!("error: {}", self.0);
        (StatusCode::INTERNAL_SERVER_ERROR, i18n::t("Something went wrong. Please try again.", "Terjadi kesalahan. Silakan coba lagi.").to_string()).into_response()
    }
}
impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError(e.to_string())
    }
}
impl From<site::SiteError> for AppError {
    fn from(e: site::SiteError) -> Self {
        AppError(e.0)
    }
}

/// The stylesheet, addressed by a hash of its content (/style.css?v=…), so browsers may keep
/// it for a year and still get the new one after a change.
async fn css() -> impl IntoResponse {
    let mut h = HeaderMap::new();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/css; charset=utf-8"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
    (h, layout::CSS)
}

async fn robots() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], security::robots_txt())
}

/// TDMRep (W3C): text and data mining rights are reserved for every page.
async fn tdmrep() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/json")], r#"[{"location":"/*","tdm-reservation":1}]"#)
}

/// Every answer within 30 seconds (the archive queries stop at 20; see QUERY_LIMIT).
async fn time_limit(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    match tokio::time::timeout(Duration::from_secs(30), next.run(req)).await {
        Ok(r) => r,
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, i18n::t("The server took too long to answer. Please try again.", "Server terlalu lama menjawab. Silakan coba lagi.").to_string()).into_response(),
    }
}

pub fn load_analysis(conn: &rusqlite::Connection) -> HashMap<String, Value> {
    let mut m = HashMap::new();
    if let Ok(mut st) = conn.prepare("SELECT key, json FROM analysis") {
        if let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
            for (k, v) in rows.flatten() {
                if let Ok(j) = serde_json::from_str(&v) {
                    m.insert(k, j);
                }
            }
        }
    }
    m
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(public::home))
        .route("/style.css", get(css))
        .route("/robots.txt", get(robots))
        .route("/.well-known/tdmrep.json", get(tdmrep))
        .route("/analysis/themes", get(public::themes))
        .route("/analysis/sources", get(public::sources))
        .route("/analysis/events", get(public::events))
        .route("/analysis/people", get(public::people))
        .route("/analysis/words", get(public::words))
        .route("/analysis/may-1998", get(public::may1998))
        .route("/analysis/1965", get(public::left1965))
        .route("/catalogue", get(public::catalogue))
        .route("/about", get(public::about))
        .route("/login", get(public::login_form).post(public::login))
        .route("/register", get(public::register_form).post(public::register))
        .route("/logout", post(public::logout))
        .route("/account", get(account::account_form).post(account::change_password))
        .route("/reset/:token", get(account::reset_form).post(account::reset))
        .route("/setup", get(account::setup_form).post(account::setup))
        .route("/read", get(read::home))
        .route("/read/search", get(read::search))
        .route("/read/m/:ak_id", get(read::message))
        .route("/read/m/:ak_id/raw", get(read::raw))
        .route("/read/m/:ak_id/bookmark", post(read::bookmark))
        .route("/read/day/:date", get(read::day))
        .route("/read/month/:ym", get(read::month))
        .route("/read/thread/:id", get(read::thread))
        .route("/read/topics", get(read::topics))
        .route("/read/lost", get(read::lost))
        .route("/read/topic", get(read::topic))
        .route("/read/source/:code", get(read::source))
        .route("/read/theme/:slug", get(read::theme))
        .route("/read/entity/:slug", get(read::entity))
        .route("/read/event/:slug", get(read::event))
        .route("/read/collection/:slug", get(read::collection))
        .route("/read/bookmarks", get(read::bookmarks))
        .route("/admin", get(admin::home))
        .route("/admin/user/:id", post(admin::update_user))
        .route("/admin/reset/:id", post(admin::reset_link))
        .route("/admin/sensitive/:ak_id", post(admin::toggle_sensitive))
        .route("/admin/review", get(review::page).post(review::decide))
        .route("/admin/checks", get(admin::checks))
        .route("/admin/checks/:key", get(admin::check_set).post(admin::save_check_set))
        .layer(axum::middleware::from_fn(time_limit))
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(axum::middleware::from_fn_with_state(state.clone(), security::context))
        .with_state(state)
}

pub struct ServeOpts {
    pub db: PathBuf,
    pub site: Option<PathBuf>,
    pub addr: String,
    pub approval: bool,
    pub secure_cookies: bool,
    pub trust_proxy: bool,
    pub base_url: Option<String>,
    pub contact: String,
    pub content: PathBuf,
    pub require_database: bool,
    pub admin_reset: Option<String>,
}

pub fn serve(o: ServeOpts) -> Result<()> {
    let addr = o.addr.clone();
    let state = state(&o)?;
    // Let go of last, after the async runtime: the PostgreSQL client closes its connections by
    // blocking, which the runtime's own threads may not do.
    let keep = state.clone();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let served = rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        eprintln!("Apakabar Archive on http://{addr}");
        axum::serve(listener, router(state).into_make_service_with_connect_info::<SocketAddr>()).with_graceful_shutdown(shutdown()).await?;
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    });
    drop(rt);
    drop(keep);
    served?;
    Ok(())
}

/// Ctrl-C, or the TERM signal a host sends before it stops or replaces the server.
async fn shutdown() {
    let term = async {
        #[cfg(unix)]
        {
            if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                s.recv().await;
            }
        }
        #[cfg(not(unix))]
        std::future::pending::<()>().await;
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = term => {},
    }
}

/// The precision checks: each set's sample from the site store (stored there the first time,
/// from the sample `analyze` drew) and the verdicts given so far.
pub fn refresh_checks(base: &Value, sc: &mut SiteConn) -> site::Res<Value> {
    let mut sets = base["sets"].as_array().cloned().unwrap_or_default();
    for s in sets.iter_mut() {
        let key = s["key"].as_str().unwrap_or("").to_string();
        let mut sample = sc.sample(&key)?;
        if sample.is_empty() {
            sample = s["sample"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|y| y.to_string())).collect()).unwrap_or_default();
            if !sample.is_empty() {
                sc.store_sample(&key, &sample)?;
            }
        }
        let mut results = serde_json::Map::new();
        for (who, (y, n, u)) in sc.verdict_counts(&key, &sample)? {
            results.insert(who, json!({"yes": y, "no": n, "unsure": u}));
        }
        s["sample"] = json!(sample);
        s["results"] = Value::Object(results);
    }
    Ok(json!({"sets": sets}))
}

/// Work out which messages readers may not open, and mark them in the archive database.
/// The site store's connection goes back to the pool before the archive work starts: a
/// database server may close a connection that sits idle, and the pool checks each one it
/// hands out.
pub fn refresh_restricted(archive: &rusqlite::Connection, site: &SiteDb) -> std::result::Result<Restricted, AppError> {
    let (decided, sensitive) = {
        let mut sc = site.get()?;
        (sc.review_decisions()?, sc.sensitive()?)
    };
    let sensitive: HashSet<String> = sensitive.into_iter().map(|x| x.0).collect();
    let tiered: Vec<String> = archive.prepare("SELECT ak_id FROM messages WHERE may98_tier >= 2")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let pending: HashSet<String> = tiered.into_iter().filter(|a| !decided.contains_key(a)).collect();
    // a text is restricted in every copy: the flags of one copy cover its duplicates
    let all = with_copies(archive, &pending.union(&sensitive).cloned().collect())?;
    db::apply_restricted(archive, &all)?;
    Ok(Restricted { pending, sensitive, all })
}

/// The AK IDs of a text: the original and every exact or near-identical copy of it.
pub fn copies_of(archive: &rusqlite::Connection, ak: &str) -> rusqlite::Result<Vec<String>> {
    let mut one = HashSet::new();
    one.insert(ak.to_string());
    let mut v: Vec<String> = with_copies(archive, &one)?.into_iter().collect();
    v.sort();
    Ok(v)
}

fn with_copies(archive: &rusqlite::Connection, aks: &HashSet<String>) -> rusqlite::Result<HashSet<String>> {
    let mut root = archive.prepare("SELECT coalesce(dup_of, id) FROM messages WHERE ak_id = ?1")?;
    let mut roots: HashSet<i64> = HashSet::new();
    for a in aks {
        if let Some(r) = root.query_row([a], |r| r.get(0)).optional()? {
            roots.insert(r);
        }
    }
    let mut members = archive.prepare("SELECT ak_id FROM messages WHERE id = ?1 UNION SELECT ak_id FROM messages WHERE dup_of = ?1")?;
    let mut out = aks.clone();
    for r in roots {
        for a in members.query_map([r], |r| r.get::<_, String>(0))? {
            out.insert(a?);
        }
    }
    Ok(out)
}

/// Recount after an editor's decision.
pub async fn reload_restricted(st: &AppState) -> std::result::Result<(), AppError> {
    let pool = st.pool.clone();
    let db = st.site.clone();
    let r = tokio::task::spawn_blocking(move || {
        let c = pool.get().map_err(|e| AppError(e.to_string()))?;
        refresh_restricted(&c, &db)
    })
    .await
    .map_err(|e| AppError(e.to_string()))??;
    *st.restricted.write().unwrap() = r;
    Ok(())
}

/// Open both stores, load the analysis, bring the first-pass checks and suggestions into the
/// site store, mark the restricted messages and, while there is no editor, print a one-time
/// link to create one. Refuses an archive database built by an older version.
pub fn state(o: &ServeOpts) -> Result<AppState> {
    let db_path = o.db.as_path();
    if !db_path.exists() {
        return Err(format!("{} not found: run `apakabar ingest` and `apakabar analyze` first", db_path.display()).into());
    }
    {
        let c = rusqlite::Connection::open(db_path)?;
        let v: Option<String> = c.query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| r.get(0)).ok();
        if v.as_deref() != Some(db::SCHEMA_VERSION) {
            return Err("this archive database was built by an older version: run `apakabar ingest` and `apakabar analyze` again (accounts are kept in the site store)".into());
        }
    }
    if o.require_database && site::env_url().is_none() {
        return Err("DATABASE_URL is not set. On this host files do not outlast a deploy, so accounts and editors' decisions must be kept in PostgreSQL: add a PostgreSQL database to the app (on Replit: Tools > Database) and deploy again.".into());
    }
    if db::ensure_start_indexes(db_path)? {
        eprintln!("archive database: added the indexes the start-up uses (once)");
    }
    layout::set_contacts(&o.contact);
    let site_db = SiteDb::for_archive(db_path, o.site.as_deref(), None)?;
    eprintln!("site store: {}", site_db.describe());
    if let Some(t) = site::tools_dir() {
        site::import_first_pass(&site_db, &t)?;
    }
    let pool = db::pool(db_path);
    // each step takes its own connection to the site store (see refresh_restricted)
    let (analysis, checks, restricted) = {
        let c = pool.get()?;
        let analysis = load_analysis(&c);
        let checks = refresh_checks(analysis.get("checks").unwrap_or(&Value::Null), &mut site_db.get()?)?;
        let restricted = refresh_restricted(&c, &site_db).map_err(|e| e.0)?;
        (analysis, checks, restricted)
    };
    let admins = site_db.get()?.count_admins()?;
    if analysis.is_empty() {
        eprintln!("warning: no analysis found; run `apakabar analyze` first");
    }
    eprintln!("{} messages wait for an editor's review; {} are restricted to researchers", restricted.pending.len(), restricted.sensitive.len());
    let base_url = o.base_url.clone().map(|b| b.trim_end_matches('/').to_string()).filter(|b| !b.is_empty());
    let base_host = base_url.as_ref().map(|b| b.trim_start_matches("https://").trim_start_matches("http://").split('/').next().unwrap_or("").to_string());
    if o.trust_proxy {
        // the load balancer's address, to read the client's address behind it
        if let Some(h) = &base_host {
            security::watch_front_door(h);
        }
    }
    if let Some(email) = o.admin_reset.as_deref().map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()) {
        let mut sc = site_db.get()?;
        match sc.account_by_email(&email)? {
            Some(a) if a.role == "admin" => {
                let t = sc.create_reset(a.id, a.id, 24)?;
                sc.audit(a.id, "reset link", "printed at start (APAKABAR_ADMIN_RESET)")?;
                let base = base_url.clone().unwrap_or_else(|| format!("http://{}", o.addr.replace("0.0.0.0", "127.0.0.1")));
                eprintln!("\nA link to set a new password for {email} (works once, for 24 hours; remove APAKABAR_ADMIN_RESET afterwards):\n  {base}/reset/{t}\n");
            }
            _ => eprintln!("APAKABAR_ADMIN_RESET: there is no editor account {email}"),
        }
    }
    let setup = if admins == 0 {
        let t = site::new_token();
        let link = match &base_url {
            Some(b) => format!("{b}/setup?token={t}"),
            None => format!("http://{}/setup?token={t}", o.addr.replace("0.0.0.0", "127.0.0.1")),
        };
        eprintln!("\nThere is no editor account yet. Create yours here (the link works once, until the server restarts):\n  {link}\n");
        Some(t)
    } else {
        None
    };
    Ok(AppState {
        pool,
        site: site_db,
        analysis: Arc::new(analysis),
        checks: Arc::new(RwLock::new(checks)),
        restricted: Arc::new(RwLock::new(restricted)),
        approval: o.approval,
        secure_cookies: o.secure_cookies,
        trust_proxy: o.trust_proxy,
        base_url,
        base_host,
        throttle: Arc::new(security::Throttle::default()),
        setup: Arc::new(Mutex::new(setup)),
        content: o.content.clone(),
    })
}

#[allow(dead_code)]
fn _paths(_: &Path) {}
