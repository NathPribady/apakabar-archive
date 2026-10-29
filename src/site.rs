//! The site store: what cannot be rebuilt from the harvested text. Accounts and sessions,
//! bookmarks and notes, the daily read counts, editors' sensitivity flags and review
//! decisions, the precision checks, password-reset links and the audit log.
//!
//! Two backends hold the same tables:
//!
//! - a SQLite file (site.db next to apakabar.db), for running the site on one computer;
//! - PostgreSQL, when `DATABASE_URL` is set. Replit (and most hosts) keep no files between
//!   deploys, so accounts there must live in a database server.
//!
//! Every query is written once, in what both understand: numbered parameters (`$1`, `$2` …,
//! rewritten to `?1`, `?2` … for SQLite), times passed in as text by the program, and
//! `ON CONFLICT` for upserts. Messages are referred to by their permanent AK ID only, so the
//! store stays valid however often the archive database is rebuilt.

use once_cell::sync::Lazy;
use r2d2::{Pool, PooledConnection};
use r2d2_postgres::PostgresConnectionManager;
use r2d2_sqlite::SqliteConnectionManager;
use regex::Regex;
use rusqlite::OptionalExtension;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_postgres_rustls::MakeRustlsConnect;

// ---------------------------------------------------------------- errors and values

#[derive(Debug)]
pub struct SiteError(pub String);
impl std::fmt::Display for SiteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SiteError {}
impl From<rusqlite::Error> for SiteError {
    fn from(e: rusqlite::Error) -> Self {
        SiteError(format!("site database: {e}"))
    }
}
impl From<postgres::Error> for SiteError {
    fn from(e: postgres::Error) -> Self {
        // the server's message and code, never its DETAIL (which can quote values, such as an
        // email address)
        match e.as_db_error() {
            Some(d) => SiteError(format!("site database: {} ({})", d.message(), d.code().code())),
            None => SiteError(format!("site database: {e}")),
        }
    }
}
impl From<r2d2::Error> for SiteError {
    fn from(e: r2d2::Error) -> Self {
        SiteError(format!("site database pool: {e}"))
    }
}
pub type Res<T> = std::result::Result<T, SiteError>;

/// A query parameter. Nulls carry their type, because PostgreSQL checks it.
#[derive(Clone, Debug)]
pub enum V {
    I(i64),
    T(String),
    NullI,
    NullT,
}
impl From<i64> for V {
    fn from(v: i64) -> Self {
        V::I(v)
    }
}
impl From<usize> for V {
    fn from(v: usize) -> Self {
        V::I(v as i64)
    }
}
impl From<bool> for V {
    fn from(v: bool) -> Self {
        V::I(v as i64)
    }
}
impl From<&str> for V {
    fn from(v: &str) -> Self {
        V::T(v.to_string())
    }
}
impl From<String> for V {
    fn from(v: String) -> Self {
        V::T(v)
    }
}
impl From<&String> for V {
    fn from(v: &String) -> Self {
        V::T(v.clone())
    }
}
impl From<Option<String>> for V {
    fn from(v: Option<String>) -> Self {
        v.map(V::T).unwrap_or(V::NullT)
    }
}
impl From<Option<i64>> for V {
    fn from(v: Option<i64>) -> Self {
        v.map(V::I).unwrap_or(V::NullI)
    }
}

/// One value of a result row.
#[derive(Clone, Debug)]
pub enum Cell {
    Null,
    I(i64),
    F(f64),
    T(String),
}

#[derive(Clone, Debug)]
pub struct Row(pub Vec<Cell>);
impl Row {
    pub fn i(&self, k: usize) -> i64 {
        self.oi(k).unwrap_or(0)
    }
    pub fn oi(&self, k: usize) -> Option<i64> {
        match self.0.get(k)? {
            Cell::I(v) => Some(*v),
            Cell::F(f) => Some(*f as i64),
            Cell::T(s) => s.parse().ok(),
            Cell::Null => None,
        }
    }
    pub fn s(&self, k: usize) -> String {
        self.os(k).unwrap_or_default()
    }
    pub fn os(&self, k: usize) -> Option<String> {
        match self.0.get(k)? {
            Cell::T(s) => Some(s.clone()),
            Cell::I(v) => Some(v.to_string()),
            Cell::F(f) => Some(f.to_string()),
            Cell::Null => None,
        }
    }
}

/// UTC time as text, "YYYY-MM-DD HH:MM:SS" (sorts as it reads; the same in both backends).
pub fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}
pub fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}
pub fn days_from_now(days: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::days(days)).format("%Y-%m-%d %H:%M:%S").to_string()
}
pub fn hours_from_now(h: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::hours(h)).format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn sha256_hex(s: &str) -> String {
    let d = Sha256::digest(s.as_bytes());
    d.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn new_token() -> String {
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ---------------------------------------------------------------- schemas

const SQLITE_SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
CREATE TABLE IF NOT EXISTS users (
  id INTEGER PRIMARY KEY, email TEXT UNIQUE NOT NULL, name TEXT, affiliation TEXT, purpose TEXT,
  pw_hash TEXT NOT NULL, role TEXT NOT NULL DEFAULT 'reader', approved INTEGER DEFAULT 1,
  created_at TEXT, last_login TEXT, terms_version TEXT, terms_accepted_at TEXT);
CREATE TABLE IF NOT EXISTS sessions (token TEXT PRIMARY KEY, user_id INTEGER, created_at TEXT, expires_at TEXT);
CREATE TABLE IF NOT EXISTS bookmarks (user_id INTEGER, ak_id TEXT, note TEXT, created_at TEXT, PRIMARY KEY(user_id, ak_id)) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS reads_log (user_id INTEGER, day TEXT, n INTEGER, PRIMARY KEY(user_id, day)) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS sensitive (ak_id TEXT PRIMARY KEY, flagged_by INTEGER, flagged_at TEXT, note TEXT);
CREATE TABLE IF NOT EXISTS checks (set_key TEXT, ak_id TEXT, reviewer TEXT, verdict TEXT, note TEXT, at TEXT, PRIMARY KEY(set_key, ak_id, reviewer)) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS check_samples (set_key TEXT, pos INTEGER, ak_id TEXT, PRIMARY KEY(set_key, pos)) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS audit (id INTEGER PRIMARY KEY, at TEXT, user_id INTEGER, action TEXT, detail TEXT);
CREATE TABLE IF NOT EXISTS review (ak_id TEXT PRIMARY KEY, status TEXT NOT NULL, by_user INTEGER, at TEXT, note TEXT);
CREATE TABLE IF NOT EXISTS review_suggestions (ak_id TEXT PRIMARY KEY, suggestion TEXT NOT NULL, reason TEXT, source TEXT, at TEXT);
CREATE TABLE IF NOT EXISTS resets (token_hash TEXT PRIMARY KEY, user_id INTEGER NOT NULL, created_by INTEGER, created_at TEXT, expires_at TEXT, used_at TEXT);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);
"#;

const PG_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS users (
  id BIGSERIAL PRIMARY KEY, email TEXT UNIQUE NOT NULL, name TEXT, affiliation TEXT, purpose TEXT,
  pw_hash TEXT NOT NULL, role TEXT NOT NULL DEFAULT 'reader', approved BIGINT DEFAULT 1,
  created_at TEXT, last_login TEXT, terms_version TEXT, terms_accepted_at TEXT);
CREATE TABLE IF NOT EXISTS sessions (token TEXT PRIMARY KEY, user_id BIGINT, created_at TEXT, expires_at TEXT);
CREATE TABLE IF NOT EXISTS bookmarks (user_id BIGINT, ak_id TEXT, note TEXT, created_at TEXT, PRIMARY KEY(user_id, ak_id));
CREATE TABLE IF NOT EXISTS reads_log (user_id BIGINT, day TEXT, n BIGINT, PRIMARY KEY(user_id, day));
CREATE TABLE IF NOT EXISTS sensitive (ak_id TEXT PRIMARY KEY, flagged_by BIGINT, flagged_at TEXT, note TEXT);
CREATE TABLE IF NOT EXISTS checks (set_key TEXT, ak_id TEXT, reviewer TEXT, verdict TEXT, note TEXT, at TEXT, PRIMARY KEY(set_key, ak_id, reviewer));
CREATE TABLE IF NOT EXISTS check_samples (set_key TEXT, pos BIGINT, ak_id TEXT, PRIMARY KEY(set_key, pos));
CREATE TABLE IF NOT EXISTS audit (id BIGSERIAL PRIMARY KEY, at TEXT, user_id BIGINT, action TEXT, detail TEXT);
CREATE TABLE IF NOT EXISTS review (ak_id TEXT PRIMARY KEY, status TEXT NOT NULL, by_user BIGINT, at TEXT, note TEXT);
CREATE TABLE IF NOT EXISTS review_suggestions (ak_id TEXT PRIMARY KEY, suggestion TEXT NOT NULL, reason TEXT, source TEXT, at TEXT);
CREATE TABLE IF NOT EXISTS resets (token_hash TEXT PRIMARY KEY, user_id BIGINT NOT NULL, created_by BIGINT, created_at TEXT, expires_at TEXT, used_at TEXT);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);
"#;

/// The reviewer under which the first-pass readings and suggestions in tools/ are filed.
pub const FIRST_PASS: &str = "first-pass";

/// Brings a store written by an earlier version up to date. Earlier versions filed the first
/// pass under another reviewer name; every reviewer that is not an editor ("user:N") is the
/// first pass. Runs at every start and changes nothing once done.
const UPGRADE: &str = "
DELETE FROM checks WHERE reviewer NOT LIKE 'user:%' AND reviewer <> 'first-pass'
  AND EXISTS (SELECT 1 FROM checks c WHERE c.set_key = checks.set_key AND c.ak_id = checks.ak_id AND c.reviewer = 'first-pass');
UPDATE checks SET reviewer = 'first-pass' WHERE reviewer NOT LIKE 'user:%' AND reviewer <> 'first-pass';
UPDATE review_suggestions SET source = 'first-pass' WHERE source IS NULL OR source <> 'first-pass';
";

// ---------------------------------------------------------------- the store

type PgManager = PostgresConnectionManager<MakeRustlsConnect>;

#[derive(Clone)]
pub enum SiteDb {
    Lite(Pool<SqliteConnectionManager>, Arc<PathBuf>),
    Pg(Pool<PgManager>),
}

/// site.db sits next to the archive database unless given explicitly.
pub fn site_path(db_path: &Path, explicit: Option<&Path>) -> PathBuf {
    match explicit {
        Some(p) => p.to_path_buf(),
        None => db_path.parent().map(|d| d.join("site.db")).unwrap_or_else(|| PathBuf::from("site.db")),
    }
}

/// The PostgreSQL address from the environment, if any (Replit sets DATABASE_URL).
pub fn env_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok().filter(|u| u.starts_with("postgres://") || u.starts_with("postgresql://"))
}

/// rustls verifies the server certificate in every mode, so the libpq modes that ask for
/// verification are read as "require" (tokio-postgres knows only disable, prefer, require).
/// Options tokio-postgres does not know (hosts add their own) are dropped rather than refused,
/// and channel binding is not insisted on: the certificate check already guards the connection.
fn pg_config(url: &str) -> Res<postgres::Config> {
    const KNOWN: &[&str] = &["user", "password", "dbname", "options", "application_name", "sslmode", "host", "hostaddr", "port", "connect_timeout", "tcp_user_timeout", "keepalives", "keepalives_idle", "target_session_attrs", "load_balance_hosts"];
    let (base, query) = match url.split_once('?') {
        Some((b, q)) => (b, q),
        None => (url, ""),
    };
    let kept: Vec<String> = query
        .split('&')
        .filter(|kv| !kv.is_empty())
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            if !KNOWN.contains(&k) {
                return None;
            }
            let v = if k == "sslmode" && (v == "verify-full" || v == "verify-ca") { "require" } else { v };
            Some(format!("{k}={v}"))
        })
        .collect();
    let url = if kept.is_empty() { base.to_string() } else { format!("{base}?{}", kept.join("&")) };
    let mut cfg = url.parse::<postgres::Config>().map_err(|e| SiteError(format!("DATABASE_URL could not be read: {e}")))?;
    // Encrypted unless the address turns it off: without `sslmode`, the client would otherwise
    // fall back to an unencrypted connection whenever TLS is refused along the way.
    let mode = kept.iter().find_map(|kv| kv.strip_prefix("sslmode="));
    if !matches!(mode, Some("disable") | Some("require")) {
        cfg.ssl_mode(postgres::config::SslMode::Require);
    }
    // Unless the address says otherwise: give up on a server that does not answer within
    // 10 seconds, and keep idle connections alive through the host's network (probes after
    // 30 seconds of quiet, not the system's 2 hours).
    if cfg.get_connect_timeout().is_none() {
        cfg.connect_timeout(std::time::Duration::from_secs(10));
    }
    if !kept.iter().any(|kv| kv.starts_with("keepalives_idle=")) {
        cfg.keepalives_idle(std::time::Duration::from_secs(30));
    }
    Ok(cfg)
}

/// TLS for the PostgreSQL connection: the server's certificate must check out against the
/// public certificate authorities, or against the authority in the file named by
/// `sslrootcert=` in DATABASE_URL (or the PGSSLROOTCERT variable), for a database with a
/// certificate of its own.
fn tls(url: &str) -> Res<MakeRustlsConnect> {
    use rustls::pki_types::{pem::PemObject, CertificateDer};
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(file) = root_cert_file(url) {
        let certs: Vec<CertificateDer> = CertificateDer::pem_file_iter(&file)
            .and_then(|it| it.collect::<Result<Vec<_>, _>>())
            .map_err(|e| SiteError(format!("the certificate file {file} could not be read: {e}")))?;
        let (added, _) = roots.add_parsable_certificates(certs);
        if added == 0 {
            return Err(SiteError(format!("no certificate found in {file}")));
        }
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(MakeRustlsConnect::new(config))
}

/// `sslrootcert=` in the address, or the PGSSLROOTCERT variable.
fn root_cert_file(url: &str) -> Option<String> {
    url.split_once('?')
        .and_then(|(_, q)| q.split('&').find_map(|kv| kv.strip_prefix("sslrootcert=").map(|v| urlencoding::decode(v).map(|c| c.into_owned()).unwrap_or_else(|_| v.to_string()))))
        .or_else(|| std::env::var("PGSSLROOTCERT").ok())
        .filter(|f| !f.is_empty() && f != "system")
}

/// A connection error, with what to do when it is the certificate check that failed.
fn connect_error(e: impl std::fmt::Display) -> SiteError {
    let msg = e.to_string();
    if msg.contains("TLS") || msg.contains("certificate") || msg.contains("tls") {
        SiteError(format!(
            "could not connect to the database in DATABASE_URL: {msg}. The database offered an encrypted connection whose certificate could not be checked. \
             Give the certificate authority of the database with sslrootcert=/path/to/ca.pem in DATABASE_URL (or PGSSLROOTCERT). \
             If the database can only be reached from inside the host's private network, sslmode=disable in DATABASE_URL connects without encryption."
        ))
    } else {
        SiteError(format!("could not connect to the database in DATABASE_URL: {msg}"))
    }
}

impl SiteDb {
    /// PostgreSQL when `url` is given, otherwise the SQLite file at `path`. Creates the tables
    /// if needed. `archive` lets a new site.db take over the accounts an older apakabar.db held.
    pub fn open(url: Option<&str>, path: &Path, archive: Option<&Path>) -> Res<SiteDb> {
        match url {
            Some(u) => {
                let mgr = PostgresConnectionManager::new(pg_config(u)?, tls(u)?);
                // Each connection is checked as it is handed out, and one the server closed is
                // replaced; few are kept open while the site is quiet.
                let pool = Pool::builder()
                    .max_size(8)
                    .min_idle(Some(1))
                    .idle_timeout(Some(std::time::Duration::from_secs(300)))
                    .test_on_check_out(true)
                    .connection_timeout(std::time::Duration::from_secs(15))
                    .build(mgr)
                    .map_err(connect_error)?;
                let mut c = pool.get().map_err(connect_error)?;
                c.batch_execute(PG_SCHEMA)?;
                c.batch_execute(UPGRADE)?;
                drop(c);
                Ok(SiteDb::Pg(pool))
            }
            None => {
                {
                    let c = rusqlite::Connection::open(path)?;
                    c.execute_batch(SQLITE_SCHEMA)?;
                    c.execute_batch(UPGRADE)?;
                    let users: i64 = c.query_row("SELECT count(*) FROM users", [], |r| r.get(0))?;
                    if users == 0 {
                        if let Some(a) = archive.filter(|a| a.exists()) {
                            migrate_old(&c, a)?;
                        }
                    }
                }
                let mgr = SqliteConnectionManager::file(path).with_init(|c| c.execute_batch("PRAGMA busy_timeout = 5000;"));
                let pool = Pool::builder().max_size(8).build(mgr)?;
                Ok(SiteDb::Lite(pool, Arc::new(path.to_path_buf())))
            }
        }
    }

    /// The store the commands use: PostgreSQL if DATABASE_URL is set (or `url` given),
    /// otherwise site.db next to the archive database (or `explicit`).
    pub fn for_archive(db_path: &Path, explicit: Option<&Path>, url: Option<String>) -> Res<SiteDb> {
        let url = url.or_else(env_url);
        SiteDb::open(url.as_deref(), &site_path(db_path, explicit), Some(db_path))
    }

    pub fn describe(&self) -> String {
        match self {
            SiteDb::Lite(_, p) => p.display().to_string(),
            SiteDb::Pg(_) => "PostgreSQL (DATABASE_URL)".into(),
        }
    }

    pub fn get(&self) -> Res<SiteConn> {
        Ok(match self {
            SiteDb::Lite(p, _) => SiteConn::Lite(p.get()?),
            SiteDb::Pg(p) => SiteConn::Pg(p.get()?),
        })
    }
}

/// An older apakabar.db kept the accounts itself; carry them over once (SQLite only).
fn migrate_old(site: &rusqlite::Connection, archive: &Path) -> rusqlite::Result<()> {
    site.execute("ATTACH DATABASE ?1 AS old", [archive.to_string_lossy()])?;
    let has: Option<String> = site.query_row("SELECT name FROM old.sqlite_master WHERE type='table' AND name='users'", [], |r| r.get(0)).optional()?;
    if has.is_some() {
        site.execute_batch(
            "INSERT OR IGNORE INTO users(id, email, name, affiliation, purpose, pw_hash, role, approved, created_at, last_login)
               SELECT id, email, name, affiliation, purpose, pw_hash, role, approved, created_at, last_login FROM old.users;
             INSERT OR IGNORE INTO reads_log SELECT user_id, day, n FROM old.reads_log;",
        )?;
        let bm: Option<String> = site.query_row("SELECT name FROM old.sqlite_master WHERE type='table' AND name='bookmarks'", [], |r| r.get(0)).optional()?;
        if bm.is_some() {
            let _ = site.execute_batch("INSERT OR IGNORE INTO bookmarks(user_id, ak_id, note, created_at) SELECT b.user_id, m.ak_id, b.note, b.created_at FROM old.bookmarks b JOIN old.messages m ON m.id = b.msg_id;");
        }
        let n: i64 = site.query_row("SELECT count(*) FROM users", [], |r| r.get(0))?;
        if n > 0 {
            eprintln!("moved {n} accounts and their bookmarks into the site database");
        }
    }
    site.execute("DETACH DATABASE old", [])?;
    Ok(())
}

// ---------------------------------------------------------------- running SQL on either backend

static NUMBERED: Lazy<Regex> = Lazy::new(|| Regex::new(r"\$(\d+)").unwrap());

fn lite_sql(sql: &str) -> String {
    NUMBERED.replace_all(sql, "?$1").into_owned()
}

fn lite_params(p: &[V]) -> Vec<rusqlite::types::Value> {
    p.iter()
        .map(|v| match v {
            V::I(i) => rusqlite::types::Value::Integer(*i),
            V::T(s) => rusqlite::types::Value::Text(s.clone()),
            V::NullI | V::NullT => rusqlite::types::Value::Null,
        })
        .collect()
}

fn lite_rows(c: &rusqlite::Connection, sql: &str, p: &[V]) -> Res<Vec<Row>> {
    let mut st = c.prepare_cached(&lite_sql(sql))?;
    let n = st.column_count();
    let rows = st.query_map(rusqlite::params_from_iter(lite_params(p)), |r| {
        let mut v = Vec::with_capacity(n);
        for k in 0..n {
            v.push(match r.get_ref(k)? {
                rusqlite::types::ValueRef::Null => Cell::Null,
                rusqlite::types::ValueRef::Integer(i) => Cell::I(i),
                rusqlite::types::ValueRef::Real(f) => Cell::F(f),
                rusqlite::types::ValueRef::Text(t) => Cell::T(String::from_utf8_lossy(t).into_owned()),
                rusqlite::types::ValueRef::Blob(b) => Cell::T(String::from_utf8_lossy(b).into_owned()),
            });
        }
        Ok(Row(v))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn lite_exec(c: &rusqlite::Connection, sql: &str, p: &[V]) -> Res<u64> {
    Ok(c.prepare_cached(&lite_sql(sql))?.execute(rusqlite::params_from_iter(lite_params(p)))? as u64)
}

type PgParam = Box<dyn postgres::types::ToSql + Sync + Send>;

fn pg_params(p: &[V]) -> Vec<PgParam> {
    p.iter()
        .map(|v| -> PgParam {
            match v {
                V::I(i) => Box::new(*i),
                V::T(s) => Box::new(s.clone()),
                V::NullI => Box::new(Option::<i64>::None),
                V::NullT => Box::new(Option::<String>::None),
            }
        })
        .collect()
}

fn pg_row(r: &postgres::Row) -> Row {
    use postgres::types::Type;
    let mut v = Vec::with_capacity(r.len());
    for (k, col) in r.columns().iter().enumerate() {
        let t = col.type_();
        let cell = if *t == Type::INT8 {
            r.get::<_, Option<i64>>(k).map(Cell::I)
        } else if *t == Type::INT4 {
            r.get::<_, Option<i32>>(k).map(|x| Cell::I(x as i64))
        } else if *t == Type::INT2 {
            r.get::<_, Option<i16>>(k).map(|x| Cell::I(x as i64))
        } else if *t == Type::BOOL {
            r.get::<_, Option<bool>>(k).map(|x| Cell::I(x as i64))
        } else if *t == Type::FLOAT8 {
            r.get::<_, Option<f64>>(k).map(Cell::F)
        } else if *t == Type::FLOAT4 {
            r.get::<_, Option<f32>>(k).map(|x| Cell::F(x as f64))
        } else {
            r.try_get::<_, Option<String>>(k).ok().flatten().map(Cell::T)
        };
        v.push(cell.unwrap_or(Cell::Null));
    }
    Row(v)
}

macro_rules! pg_refs {
    ($b:expr) => {
        $b.iter().map(|x| x.as_ref() as &(dyn postgres::types::ToSql + Sync)).collect::<Vec<_>>()
    };
}

pub enum SiteConn {
    Lite(PooledConnection<SqliteConnectionManager>),
    Pg(PooledConnection<PgManager>),
}

/// A transaction on either backend, with the same three calls as a connection.
pub enum Tx<'a> {
    Lite(rusqlite::Transaction<'a>),
    Pg(postgres::Transaction<'a>),
}

impl Tx<'_> {
    pub fn query(&mut self, sql: &str, p: &[V]) -> Res<Vec<Row>> {
        match self {
            Tx::Lite(t) => lite_rows(t, sql, p),
            Tx::Pg(t) => {
                let b = pg_params(p);
                Ok(t.query(sql, &pg_refs!(b))?.iter().map(pg_row).collect())
            }
        }
    }
    pub fn exec(&mut self, sql: &str, p: &[V]) -> Res<u64> {
        match self {
            Tx::Lite(t) => lite_exec(t, sql, p),
            Tx::Pg(t) => {
                let b = pg_params(p);
                Ok(t.execute(sql, &pg_refs!(b))?)
            }
        }
    }
    pub fn one(&mut self, sql: &str, p: &[V]) -> Res<Option<Row>> {
        Ok(self.query(sql, p)?.into_iter().next())
    }
}

impl SiteConn {
    pub fn query(&mut self, sql: &str, p: &[V]) -> Res<Vec<Row>> {
        match self {
            SiteConn::Lite(c) => lite_rows(c, sql, p),
            SiteConn::Pg(c) => {
                let b = pg_params(p);
                Ok(c.query(sql, &pg_refs!(b))?.iter().map(pg_row).collect())
            }
        }
    }
    pub fn one(&mut self, sql: &str, p: &[V]) -> Res<Option<Row>> {
        Ok(self.query(sql, p)?.into_iter().next())
    }
    pub fn exec(&mut self, sql: &str, p: &[V]) -> Res<u64> {
        match self {
            SiteConn::Lite(c) => lite_exec(c, sql, p),
            SiteConn::Pg(c) => {
                let b = pg_params(p);
                Ok(c.execute(sql, &pg_refs!(b))?)
            }
        }
    }
    pub fn scalar(&mut self, sql: &str, p: &[V]) -> Res<i64> {
        Ok(self.one(sql, p)?.map(|r| r.i(0)).unwrap_or(0))
    }
    /// Run `f` in a transaction: all of its writes land, or none.
    pub fn tx<T>(&mut self, f: impl FnOnce(&mut Tx) -> Res<T>) -> Res<T> {
        match self {
            SiteConn::Lite(c) => {
                let t = c.transaction()?;
                let mut tx = Tx::Lite(t);
                let out = f(&mut tx)?;
                if let Tx::Lite(t) = tx {
                    t.commit()?;
                }
                Ok(out)
            }
            SiteConn::Pg(c) => {
                let t = c.transaction()?;
                let mut tx = Tx::Pg(t);
                let out = f(&mut tx)?;
                if let Tx::Pg(t) = tx {
                    t.commit()?;
                }
                Ok(out)
            }
        }
    }
}

/// "($1,$2,$3),($4,$5,$6)…" for a multi-row insert of `rows` rows of `cols` values.
fn values_list(rows: usize, cols: usize) -> String {
    (0..rows).map(|r| format!("({})", (0..cols).map(|c| format!("${}", r * cols + c + 1)).collect::<Vec<_>>().join(","))).collect::<Vec<_>>().join(",")
}

// ---------------------------------------------------------------- accounts and sessions

#[derive(Clone, Debug)]
pub struct Account {
    pub id: i64,
    pub email: String,
    pub name: String,
    pub role: String,
    pub approved: bool,
    pub pw_hash: String,
}

#[derive(Clone, Debug)]
pub struct AccountRow {
    pub id: i64,
    pub email: String,
    pub name: String,
    pub affiliation: String,
    pub purpose: String,
    pub role: String,
    pub approved: bool,
    pub created_at: String,
    pub last_login: Option<String>,
}

pub const SESSION_DAYS: i64 = 30;

impl SiteConn {
    pub fn account_by_email(&mut self, email: &str) -> Res<Option<Account>> {
        Ok(self
            .one("SELECT id, email, coalesce(name,''), role, approved, pw_hash FROM users WHERE email = $1", &[email.into()])?
            .map(|r| Account { id: r.i(0), email: r.s(1), name: r.s(2), role: r.s(3), approved: r.i(4) == 1, pw_hash: r.s(5) }))
    }

    pub fn account_by_id(&mut self, id: i64) -> Res<Option<Account>> {
        Ok(self
            .one("SELECT id, email, coalesce(name,''), role, approved, pw_hash FROM users WHERE id = $1", &[id.into()])?
            .map(|r| Account { id: r.i(0), email: r.s(1), name: r.s(2), role: r.s(3), approved: r.i(4) == 1, pw_hash: r.s(5) }))
    }

    /// A new reader from the registration form; returns the new ID.
    #[allow(clippy::too_many_arguments)]
    pub fn create_account(&mut self, email: &str, name: &str, affiliation: &str, purpose: &str, pw_hash: &str, role: &str, approved: bool, terms: Option<&str>) -> Res<i64> {
        let t = now();
        let r = self.one(
            "INSERT INTO users(email, name, affiliation, purpose, pw_hash, role, approved, created_at, terms_version, terms_accepted_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
            &[email.into(), name.into(), affiliation.into(), purpose.into(), pw_hash.into(), role.into(), approved.into(), t.clone().into(), terms.map(|s| s.to_string()).into(), terms.map(|_| t.clone()).into()],
        )?;
        Ok(r.map(|r| r.i(0)).unwrap_or(0))
    }

    /// Create or update an account from the command line or the setup page.
    pub fn upsert_account(&mut self, email: &str, name: &str, pw_hash: &str, role: &str) -> Res<()> {
        self.exec(
            "INSERT INTO users(email, name, pw_hash, role, approved, created_at) VALUES ($1, $2, $3, $4, 1, $5)
             ON CONFLICT(email) DO UPDATE SET name = excluded.name, pw_hash = excluded.pw_hash, role = excluded.role, approved = 1",
            &[email.into(), name.into(), pw_hash.into(), role.into(), now().into()],
        )?;
        Ok(())
    }

    pub fn count_admins(&mut self) -> Res<i64> {
        self.scalar("SELECT count(*) FROM users WHERE role = 'admin' AND approved = 1", &[])
    }

    pub fn list_accounts(&mut self) -> Res<Vec<AccountRow>> {
        Ok(self
            .query("SELECT id, email, coalesce(name,''), coalesce(affiliation,''), coalesce(purpose,''), role, approved, coalesce(created_at,''), last_login FROM users ORDER BY approved, created_at DESC", &[])?
            .into_iter()
            .map(|r| AccountRow { id: r.i(0), email: r.s(1), name: r.s(2), affiliation: r.s(3), purpose: r.s(4), role: r.s(5), approved: r.i(6) == 1, created_at: r.s(7), last_login: r.os(8) })
            .collect())
    }

    pub fn update_account(&mut self, id: i64, role: &str, approved: bool) -> Res<()> {
        self.exec("UPDATE users SET role = $1, approved = $2 WHERE id = $3", &[role.into(), approved.into(), id.into()])?;
        if !approved {
            self.exec("DELETE FROM sessions WHERE user_id = $1", &[id.into()])?;
        }
        Ok(())
    }

    pub fn set_password(&mut self, id: i64, pw_hash: &str) -> Res<()> {
        self.exec("UPDATE users SET pw_hash = $1 WHERE id = $2", &[pw_hash.into(), id.into()])?;
        Ok(())
    }

    /// A new session; the cookie holds the token, the store only its hash.
    pub fn create_session(&mut self, user_id: i64) -> Res<String> {
        let t = new_token();
        let n = now();
        self.exec("INSERT INTO sessions(token, user_id, created_at, expires_at) VALUES ($1, $2, $3, $4)", &[sha256_hex(&t).into(), user_id.into(), n.clone().into(), days_from_now(SESSION_DAYS).into()])?;
        self.exec("UPDATE users SET last_login = $1 WHERE id = $2", &[n.clone().into(), user_id.into()])?;
        self.exec("DELETE FROM sessions WHERE expires_at < $1", &[n.into()])?;
        Ok(t)
    }

    /// The approved account a session token belongs to.
    pub fn account_for_session(&mut self, token: &str) -> Res<Option<Account>> {
        Ok(self
            .one(
                "SELECT u.id, u.email, coalesce(u.name,''), u.role, u.approved, u.pw_hash FROM sessions s JOIN users u ON u.id = s.user_id
                 WHERE s.token = $1 AND s.expires_at > $2 AND u.approved = 1",
                &[sha256_hex(token).into(), now().into()],
            )?
            .map(|r| Account { id: r.i(0), email: r.s(1), name: r.s(2), role: r.s(3), approved: r.i(4) == 1, pw_hash: r.s(5) }))
    }

    pub fn delete_session(&mut self, token: &str) -> Res<()> {
        self.exec("DELETE FROM sessions WHERE token = $1", &[sha256_hex(token).into()])?;
        Ok(())
    }

    pub fn delete_sessions_of(&mut self, user_id: i64) -> Res<()> {
        self.exec("DELETE FROM sessions WHERE user_id = $1", &[user_id.into()])?;
        Ok(())
    }

    // ------------------------------------------------------------ password-reset links

    /// A one-time link an editor sends to a reader by hand; valid for `hours`.
    pub fn create_reset(&mut self, user_id: i64, by: i64, hours: i64) -> Res<String> {
        let t = new_token();
        self.exec(
            "INSERT INTO resets(token_hash, user_id, created_by, created_at, expires_at) VALUES ($1, $2, $3, $4, $5)",
            &[sha256_hex(&t).into(), user_id.into(), by.into(), now().into(), hours_from_now(hours).into()],
        )?;
        Ok(t)
    }

    /// The account a reset token opens, if the token is unused and not expired.
    pub fn reset_account(&mut self, token: &str) -> Res<Option<Account>> {
        let r = self.one("SELECT user_id FROM resets WHERE token_hash = $1 AND used_at IS NULL AND expires_at > $2", &[sha256_hex(token).into(), now().into()])?;
        match r {
            Some(r) => self.account_by_id(r.i(0)),
            None => Ok(None),
        }
    }

    /// Set a new password through a reset token; the token is spent and every session ends.
    pub fn use_reset(&mut self, token: &str, pw_hash: &str) -> Res<Option<i64>> {
        let h = sha256_hex(token);
        let n = now();
        self.tx(|t| {
            let r = t.one("SELECT user_id FROM resets WHERE token_hash = $1 AND used_at IS NULL AND expires_at > $2", &[h.clone().into(), n.clone().into()])?;
            let uid = match r {
                Some(r) => r.i(0),
                None => return Ok(None),
            };
            // spent once: a second request with the same token at the same moment changes nothing
            if t.exec("UPDATE resets SET used_at = $1 WHERE token_hash = $2 AND used_at IS NULL", &[n.clone().into(), h.clone().into()])? == 0 {
                return Ok(None);
            }
            t.exec("UPDATE users SET pw_hash = $1 WHERE id = $2", &[pw_hash.into(), uid.into()])?;
            t.exec("DELETE FROM sessions WHERE user_id = $1", &[uid.into()])?;
            Ok(Some(uid))
        })
    }

    // ------------------------------------------------------------ bookmarks and reading

    pub fn bookmark_count(&mut self, uid: i64) -> Res<i64> {
        self.scalar("SELECT count(*) FROM bookmarks WHERE user_id = $1", &[uid.into()])
    }

    pub fn bookmark_note(&mut self, uid: i64, ak: &str) -> Res<Option<String>> {
        Ok(self.one("SELECT coalesce(note,'') FROM bookmarks WHERE user_id = $1 AND ak_id = $2", &[uid.into(), ak.into()])?.map(|r| r.s(0)))
    }

    pub fn save_bookmark(&mut self, uid: i64, ak: &str, note: &str) -> Res<()> {
        self.exec(
            "INSERT INTO bookmarks(user_id, ak_id, note, created_at) VALUES ($1, $2, $3, $4) ON CONFLICT(user_id, ak_id) DO UPDATE SET note = excluded.note",
            &[uid.into(), ak.into(), note.into(), now().into()],
        )?;
        Ok(())
    }

    pub fn delete_bookmark(&mut self, uid: i64, ak: &str) -> Res<()> {
        self.exec("DELETE FROM bookmarks WHERE user_id = $1 AND ak_id = $2", &[uid.into(), ak.into()])?;
        Ok(())
    }

    /// (AK ID, note) of every bookmark of a reader.
    pub fn bookmarks(&mut self, uid: i64) -> Res<Vec<(String, String)>> {
        Ok(self.query("SELECT ak_id, coalesce(note,'') FROM bookmarks WHERE user_id = $1 ORDER BY ak_id", &[uid.into()])?.into_iter().map(|r| (r.s(0), r.s(1))).collect())
    }

    /// Count one more message read today; returns today's total for the reader.
    pub fn count_read(&mut self, uid: i64) -> Res<i64> {
        let d = today();
        self.exec(
            "INSERT INTO reads_log(user_id, day, n) VALUES ($1, $2, 1) ON CONFLICT(user_id, day) DO UPDATE SET n = reads_log.n + 1",
            &[uid.into(), d.clone().into()],
        )?;
        self.scalar("SELECT n FROM reads_log WHERE user_id = $1 AND day = $2", &[uid.into(), d.into()])
    }

    pub fn reads_since(&mut self, day: &str) -> Res<i64> {
        self.scalar("SELECT CAST(coalesce(sum(n), 0) AS BIGINT) FROM reads_log WHERE day >= $1", &[day.into()])
    }

    // ------------------------------------------------------------ sensitivity and review

    /// Messages the editors restricted to researchers: (AK ID, when).
    pub fn sensitive(&mut self) -> Res<Vec<(String, String)>> {
        Ok(self.query("SELECT ak_id, coalesce(flagged_at,'') FROM sensitive ORDER BY ak_id", &[])?.into_iter().map(|r| (r.s(0), r.s(1))).collect())
    }

    pub fn set_sensitive(&mut self, aks: &[String], uid: i64, on: bool, note: &str) -> Res<()> {
        let n = now();
        self.tx(|t| {
            for a in aks {
                if on {
                    t.exec("INSERT INTO sensitive(ak_id, flagged_by, flagged_at, note) VALUES ($1, $2, $3, $4) ON CONFLICT(ak_id) DO NOTHING", &[a.into(), uid.into(), n.clone().into(), note.into()])?;
                } else {
                    t.exec("DELETE FROM sensitive WHERE ak_id = $1", &[a.into()])?;
                }
            }
            Ok(())
        })
    }

    /// Editors' review decisions: AK ID -> "cleared" or "restricted".
    pub fn review_decisions(&mut self) -> Res<HashMap<String, (String, i64, String)>> {
        Ok(self.query("SELECT ak_id, status, coalesce(by_user, 0), coalesce(at,'') FROM review", &[])?.into_iter().map(|r| (r.s(0), (r.s(1), r.i(2), r.s(3)))).collect())
    }

    /// Record a review decision for messages (a message and its copies). "restricted" also
    /// marks them sensitive; "cleared" lifts any sensitivity flag; "pending" undoes a decision.
    pub fn set_review(&mut self, aks: &[String], status: &str, uid: i64) -> Res<()> {
        let n = now();
        self.tx(|t| {
            for a in aks {
                if status == "pending" {
                    t.exec("DELETE FROM review WHERE ak_id = $1", &[a.into()])?;
                    t.exec("DELETE FROM sensitive WHERE ak_id = $1 AND note = 'review'", &[a.into()])?;
                    continue;
                }
                t.exec(
                    "INSERT INTO review(ak_id, status, by_user, at) VALUES ($1, $2, $3, $4) ON CONFLICT(ak_id) DO UPDATE SET status = excluded.status, by_user = excluded.by_user, at = excluded.at",
                    &[a.into(), status.into(), uid.into(), n.clone().into()],
                )?;
                if status == "restricted" {
                    t.exec("INSERT INTO sensitive(ak_id, flagged_by, flagged_at, note) VALUES ($1, $2, $3, 'review') ON CONFLICT(ak_id) DO NOTHING", &[a.into(), uid.into(), n.clone().into()])?;
                } else {
                    t.exec("DELETE FROM sensitive WHERE ak_id = $1", &[a.into()])?;
                }
            }
            Ok(())
        })
    }

    /// The first-pass suggestions: AK ID -> (suggestion, reason).
    pub fn review_suggestions(&mut self) -> Res<HashMap<String, (String, String)>> {
        Ok(self.query("SELECT ak_id, suggestion, coalesce(reason,'') FROM review_suggestions", &[])?.into_iter().map(|r| (r.s(0), (r.s(1), r.s(2)))).collect())
    }

    // ------------------------------------------------------------ precision checks

    pub fn sample(&mut self, set_key: &str) -> Res<Vec<String>> {
        Ok(self.query("SELECT ak_id FROM check_samples WHERE set_key = $1 ORDER BY pos", &[set_key.into()])?.into_iter().map(|r| r.s(0)).collect())
    }

    pub fn store_sample(&mut self, set_key: &str, ids: &[String]) -> Res<()> {
        self.tx(|t| {
            for (k, a) in ids.iter().enumerate() {
                t.exec("INSERT INTO check_samples(set_key, pos, ak_id) VALUES ($1, $2, $3) ON CONFLICT(set_key, pos) DO NOTHING", &[set_key.into(), k.into(), a.into()])?;
            }
            Ok(())
        })
    }

    /// Verdict counts per reviewer over a sample: reviewer -> (yes, no, unsure).
    pub fn verdict_counts(&mut self, set_key: &str, sample: &[String]) -> Res<HashMap<String, (i64, i64, i64)>> {
        let mut out: HashMap<String, (i64, i64, i64)> = HashMap::new();
        if sample.is_empty() {
            return Ok(out);
        }
        let inner = (0..sample.len()).map(|k| format!("${}", k + 2)).collect::<Vec<_>>().join(",");
        let mut p: Vec<V> = vec![set_key.into()];
        p.extend(sample.iter().map(V::from));
        for r in self.query(&format!("SELECT reviewer, verdict, count(*) FROM checks WHERE set_key = $1 AND ak_id IN ({inner}) GROUP BY reviewer, verdict"), &p)? {
            let e = out.entry(r.s(0)).or_default();
            match r.s(1).as_str() {
                "yes" => e.0 += r.i(2),
                "no" => e.1 += r.i(2),
                _ => e.2 += r.i(2),
            }
        }
        Ok(out)
    }

    /// (verdict, note) of one reviewer for one message of a set.
    pub fn verdict(&mut self, set_key: &str, ak: &str, reviewer: &str) -> Res<Option<(String, String)>> {
        Ok(self.one("SELECT verdict, coalesce(note,'') FROM checks WHERE set_key = $1 AND ak_id = $2 AND reviewer = $3", &[set_key.into(), ak.into(), reviewer.into()])?.map(|r| (r.s(0), r.s(1))))
    }

    pub fn save_verdicts(&mut self, set_key: &str, reviewer: &str, verdicts: &[(String, String)]) -> Res<usize> {
        let n = now();
        self.tx(|t| {
            for (ak, v) in verdicts {
                t.exec(
                    "INSERT INTO checks(set_key, ak_id, reviewer, verdict, at) VALUES ($1, $2, $3, $4, $5)
                     ON CONFLICT(set_key, ak_id, reviewer) DO UPDATE SET verdict = excluded.verdict, at = excluded.at",
                    &[set_key.into(), ak.into(), reviewer.into(), v.into(), n.clone().into()],
                )?;
            }
            Ok(verdicts.len())
        })
    }

    // ------------------------------------------------------------ audit log and settings

    pub fn audit(&mut self, uid: i64, action: &str, detail: &str) -> Res<()> {
        self.exec("INSERT INTO audit(at, user_id, action, detail) VALUES ($1, $2, $3, $4)", &[now().into(), uid.into(), action.into(), detail.into()])?;
        Ok(())
    }

    /// The latest editorial actions: (when, who, action, detail).
    pub fn recent_audit(&mut self, n: i64) -> Res<Vec<(String, String, String, String)>> {
        Ok(self
            .query("SELECT a.at, coalesce(u.email, '?'), a.action, coalesce(a.detail, '') FROM audit a LEFT JOIN users u ON u.id = a.user_id ORDER BY a.id DESC LIMIT $1", &[n.into()])?
            .into_iter()
            .map(|r| (r.s(0), r.s(1), r.s(2), r.s(3)))
            .collect())
    }

    pub fn meta(&mut self, key: &str) -> Res<Option<String>> {
        Ok(self.one("SELECT value FROM meta WHERE key = $1", &[key.into()])?.and_then(|r| r.os(0)))
    }

    pub fn set_meta(&mut self, key: &str, value: &str) -> Res<()> {
        self.exec("INSERT INTO meta(key, value) VALUES ($1, $2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", &[key.into(), value.into()])?;
        Ok(())
    }

    // ------------------------------------------------------------ first-pass imports

    /// Load the first-pass precision checks (samples and verdicts, reviewer
    /// FIRST_PASS) from tools/first_pass_checks.json. Existing rows are kept, so an
    /// editor's verdicts are never touched and a second run changes nothing.
    pub fn import_first_pass_checks(&mut self, data: &Value) -> Res<(usize, usize)> {
        let mut samples = 0;
        let mut verdicts = 0;
        let sets: Vec<(String, Vec<String>)> = data["samples"]
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default())).collect())
            .unwrap_or_default();
        let rows: Vec<[String; 4]> = data["verdicts"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| Some([v["set"].as_str()?.to_string(), v["ak_id"].as_str()?.to_string(), v["verdict"].as_str()?.to_string(), v["note"].as_str().unwrap_or("").to_string()]))
                    .collect()
            })
            .unwrap_or_default();
        self.tx(|t| {
            for (key, ids) in &sets {
                if t.one("SELECT 1 FROM check_samples WHERE set_key = $1 LIMIT 1", &[key.into()])?.is_some() {
                    continue; // a sample is never redrawn
                }
                for chunk in ids.chunks(100).enumerate() {
                    let (c, part) = chunk;
                    let mut p: Vec<V> = Vec::new();
                    for (k, a) in part.iter().enumerate() {
                        p.push(key.into());
                        p.push((c * 100 + k).into());
                        p.push(a.into());
                    }
                    t.exec(&format!("INSERT INTO check_samples(set_key, pos, ak_id) VALUES {} ON CONFLICT DO NOTHING", values_list(part.len(), 3)), &p)?;
                }
                samples += 1;
            }
            for part in rows.chunks(200) {
                let mut p: Vec<V> = Vec::new();
                for r in part {
                    p.push((&r[0]).into());
                    p.push((&r[1]).into());
                    p.push(FIRST_PASS.into());
                    p.push((&r[2]).into());
                    p.push((&r[3]).into());
                    p.push("2026-10-28".into());
                }
                verdicts += t.exec(&format!("INSERT INTO checks(set_key, ak_id, reviewer, verdict, note, at) VALUES {} ON CONFLICT DO NOTHING", values_list(part.len(), 6)), &p)? as usize;
            }
            Ok(())
        })?;
        Ok((samples, verdicts))
    }

    /// Load the first-pass privacy suggestions (tools/first_pass_review.json): for each
    /// message "restrict" or "show" and a one-line reason. Existing rows are kept.
    pub fn import_review_suggestions(&mut self, data: &Value) -> Res<usize> {
        let rows: Vec<[String; 3]> = data["suggestions"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| Some([v["ak_id"].as_str()?.to_string(), v["suggestion"].as_str()?.to_string(), v["reason"].as_str().unwrap_or("").to_string()])).collect())
            .unwrap_or_default();
        let mut n = 0;
        self.tx(|t| {
            for part in rows.chunks(200) {
                let mut p: Vec<V> = Vec::new();
                for r in part {
                    p.push((&r[0]).into());
                    p.push((&r[1]).into());
                    p.push((&r[2]).into());
                    p.push(FIRST_PASS.into());
                    p.push("2026-10-28".into());
                }
                n += t.exec(&format!("INSERT INTO review_suggestions(ak_id, suggestion, reason, source, at) VALUES {} ON CONFLICT DO NOTHING", values_list(part.len(), 5)), &p)? as usize;
            }
            Ok(())
        })?;
        Ok(n)
    }
}

/// The program's tools/ folder: next to the working directory, or two levels above the
/// binary (target/release/apakabar).
pub fn tools_dir() -> Option<PathBuf> {
    let mut c = vec![PathBuf::from("tools")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(root) = exe.parent().and_then(|p| p.parent()).and_then(|p| p.parent()) {
            c.push(root.join("tools"));
        }
    }
    c.into_iter().find(|p| p.join("first_pass_checks.json").exists())
}

/// Load the first-pass files shipped in tools/ into the store, once per file version.
/// Missing files are skipped quietly (a store can run without them).
pub fn import_first_pass(site: &SiteDb, tools: &Path) -> Res<()> {
    let mut c = site.get()?;
    for (file, key) in [("first_pass_checks.json", "imported:first_pass_checks"), ("first_pass_review.json", "imported:first_pass_review")] {
        let p = tools.join(file);
        let bytes = match std::fs::read(&p) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let digest = {
            let d = Sha256::digest(&bytes);
            d.iter().take(8).map(|b| format!("{b:02x}")).collect::<String>()
        };
        if c.meta(key)?.as_deref() == Some(digest.as_str()) {
            continue;
        }
        let v: Value = serde_json::from_slice(&bytes).map_err(|e| SiteError(format!("{}: {e}", p.display())))?;
        if file == "first_pass_checks.json" {
            let (s, n) = c.import_first_pass_checks(&v)?;
            eprintln!("first-pass precision checks: {s} samples and {n} verdicts added");
        } else {
            let n = c.import_review_suggestions(&v)?;
            eprintln!("first-pass privacy suggestions: {n} added");
        }
        c.set_meta(key, &digest)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// The PostgreSQL tests share one database (and one of them closes every connection to
    /// it), so they take turns.
    static PG_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    use super::*;

    fn roundtrip(db: &SiteDb) {
        let mut c = db.get().unwrap();
        // accounts and sessions
        let id = c.create_account("a@example.org", "A", "Uni", "research", "hash", "reader", false, Some("2026-10")).unwrap();
        assert!(id > 0);
        assert!(c.account_for_session("nope").unwrap().is_none());
        c.update_account(id, "researcher", true).unwrap();
        let t = c.create_session(id).unwrap();
        let a = c.account_for_session(&t).unwrap().unwrap();
        assert_eq!((a.id, a.role.as_str()), (id, "researcher"));
        c.upsert_account("a@example.org", "A2", "hash2", "admin").unwrap();
        assert_eq!(c.count_admins().unwrap(), 1);
        // reads, bookmarks
        assert_eq!(c.count_read(id).unwrap(), 1);
        assert_eq!(c.count_read(id).unwrap(), 2);
        assert_eq!(c.reads_since("2000-01-01").unwrap(), 2);
        c.save_bookmark(id, "AK-1998-05-15-0001", "note").unwrap();
        c.save_bookmark(id, "AK-1998-05-15-0001", "note 2").unwrap();
        assert_eq!(c.bookmark_note(id, "AK-1998-05-15-0001").unwrap().as_deref(), Some("note 2"));
        assert_eq!(c.bookmark_count(id).unwrap(), 1);
        // review and sensitivity
        let aks = vec!["AK-1998-05-15-0002".to_string(), "AKL-x-0001".to_string()];
        c.set_review(&aks, "restricted", id).unwrap();
        assert_eq!(c.sensitive().unwrap().len(), 2);
        c.set_review(&aks[..1], "cleared", id).unwrap();
        assert_eq!(c.sensitive().unwrap().len(), 1);
        assert_eq!(c.review_decisions().unwrap()["AK-1998-05-15-0002"].0, "cleared");
        // checks
        let data: Value = serde_json::json!({"samples": {"may98:3/3": ["AK-1", "AK-2"]}, "verdicts": [{"set": "may98:3/3", "ak_id": "AK-1", "verdict": "yes"}, {"set": "may98:3/3", "ak_id": "AK-2", "verdict": "no", "note": "n"}]});
        assert_eq!(c.import_first_pass_checks(&data).unwrap(), (1, 2));
        assert_eq!(c.import_first_pass_checks(&data).unwrap(), (0, 0));
        assert_eq!(c.sample("may98:3/3").unwrap(), vec!["AK-1", "AK-2"]);
        c.save_verdicts("may98:3/3", "user:1", &[("AK-1".into(), "unsure".into())]).unwrap();
        let counts = c.verdict_counts("may98:3/3", &["AK-1".to_string(), "AK-2".to_string()]).unwrap();
        assert_eq!(counts[FIRST_PASS], (1, 1, 0));
        assert_eq!(counts["user:1"], (0, 0, 1));
        // resets
        let tok = c.create_reset(id, id, 48).unwrap();
        assert!(c.reset_account(&tok).unwrap().is_some());
        assert_eq!(c.use_reset(&tok, "hash3").unwrap(), Some(id));
        assert!(c.use_reset(&tok, "hash4").unwrap().is_none());
        assert!(c.account_for_session(&t).unwrap().is_none());
        // audit and meta
        c.audit(id, "test", "detail").unwrap();
        assert_eq!(c.recent_audit(5).unwrap()[0].2, "test");
        c.set_meta("k", "v").unwrap();
        c.set_meta("k", "w").unwrap();
        assert_eq!(c.meta("k").unwrap().as_deref(), Some("w"));
    }

    /// The first pass filed under an earlier reviewer name is renamed; editors' verdicts stay.
    fn upgrade(db: &SiteDb) {
        let mut c = db.get().unwrap();
        for (ak, who) in [("AK-u1", "earlier-name"), ("AK-u1", "user:7"), ("AK-u2", "earlier-name"), ("AK-u2", FIRST_PASS)] {
            c.exec("INSERT INTO checks(set_key, ak_id, reviewer, verdict, at) VALUES ('u', $1, $2, 'yes', '2026-10-28')", &[ak.into(), who.into()]).unwrap();
        }
        c.exec("INSERT INTO review_suggestions(ak_id, suggestion, reason, source, at) VALUES ('AK-u1', 'show', '', 'earlier-name', '2026-10-28')", &[]).unwrap();
        match &mut c {
            SiteConn::Lite(x) => x.execute_batch(UPGRADE).unwrap(),
            SiteConn::Pg(x) => x.batch_execute(UPGRADE).unwrap(),
        }
        let who: Vec<String> = c.query("SELECT reviewer FROM checks WHERE set_key = 'u' ORDER BY ak_id, reviewer", &[]).unwrap().iter().map(|r| r.s(0)).collect();
        assert_eq!(who, vec![FIRST_PASS, "user:7", FIRST_PASS]);
        assert_eq!(c.scalar("SELECT count(*) FROM review_suggestions WHERE source = 'first-pass' AND ak_id = 'AK-u1'", &[]).unwrap(), 1);
    }

    #[test]
    fn sqlite_store() {
        let dir = std::env::temp_dir().join(format!("apakabar-site-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("site.db");
        let _ = std::fs::remove_file(&p);
        let db = SiteDb::open(None, &p, None).unwrap();
        roundtrip(&db);
        upgrade(&db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Runs against a real server when TEST_DATABASE_URL is set, e.g.
    /// postgres://apakabar:apakabar@127.0.0.1/apakabar_test?sslmode=disable
    #[test]
    fn postgres_store() {
        let Ok(url) = std::env::var("TEST_DATABASE_URL") else { return };
        let _one_at_a_time = PG_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        {
            let mut cfg = pg_config(&url).unwrap();
            let mut c = cfg.connect(postgres::NoTls).unwrap();
            c.batch_execute("DROP TABLE IF EXISTS users, sessions, bookmarks, reads_log, sensitive, checks, check_samples, audit, review, review_suggestions, resets, meta").unwrap();
            let _ = &mut cfg;
        }
        let db = SiteDb::open(Some(&url), Path::new("unused"), None).unwrap();
        roundtrip(&db);
        upgrade(&db);
    }

    #[test]
    fn database_urls() {
        let c = pg_config("postgresql://u:p@ep-x.neon.tech/neondb?sslmode=verify-full&channel_binding=require&foo=1").unwrap();
        assert_eq!(c.get_ssl_mode(), postgres::config::SslMode::Require);
        assert_eq!(c.get_dbname(), Some("neondb"));
        assert!(pg_config("postgres://u:p@localhost:5432/db").is_ok());
        // TLS is required unless the address says sslmode=disable
        use postgres::config::SslMode;
        assert_eq!(pg_config("postgres://u:p@h/db").unwrap().get_ssl_mode(), SslMode::Require);
        assert_eq!(pg_config("postgres://u:p@h/db?sslmode=prefer").unwrap().get_ssl_mode(), SslMode::Require);
        assert_eq!(pg_config("postgres://u:p@h/db?sslmode=disable").unwrap().get_ssl_mode(), SslMode::Disable);
        // time limits: ours by default, the address's when it gives them
        use std::time::Duration;
        assert_eq!(c.get_connect_timeout(), Some(&Duration::from_secs(10)));
        assert_eq!(c.get_keepalives_idle(), Duration::from_secs(30));
        let c = pg_config("postgres://u@h/db?connect_timeout=3&keepalives_idle=5").unwrap();
        assert_eq!(c.get_connect_timeout(), Some(&Duration::from_secs(3)));
        assert_eq!(c.get_keepalives_idle(), Duration::from_secs(5));
    }

    /// A server that closes the store's connections (as a host's database may do with idle
    /// ones) costs nothing but a new connection. Runs when TEST_DATABASE_URL is set.
    #[test]
    fn postgres_reconnects() {
        let Ok(url) = std::env::var("TEST_DATABASE_URL") else { return };
        let _one_at_a_time = PG_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let db = SiteDb::open(Some(&url), Path::new("unused"), None).unwrap();
        let mut held = db.get().unwrap();
        held.count_admins().unwrap();
        let mut admin = pg_config(&url).unwrap().connect(postgres::NoTls).unwrap();
        let closed: i64 = admin
            .query_one("SELECT count(*) FROM (SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid()) t", &[])
            .unwrap()
            .get(0);
        assert!(closed >= 1);
        std::thread::sleep(std::time::Duration::from_millis(200));
        // a connection held across the closing fails, as before …
        assert!(held.count_admins().is_err());
        drop(held);
        // … but the next one the pool hands out works
        db.get().unwrap().count_admins().unwrap();
        db.get().unwrap().count_admins().unwrap();
    }

    #[test]
    fn root_certificates() {
        assert_eq!(root_cert_file("postgres://u@h/db?sslmode=require&sslrootcert=%2Fetc%2Fca.pem").as_deref(), Some("/etc/ca.pem"));
        assert_eq!(root_cert_file("postgres://u@h/db?sslrootcert=system"), std::env::var("PGSSLROOTCERT").ok().filter(|f| !f.is_empty() && f != "system"));
        assert!(tls("postgres://u@h/db?sslrootcert=/nonexistent.pem").is_err());
        assert!(tls("postgres://u@h/db").is_ok());
    }

    #[test]
    fn numbered_parameters() {
        assert_eq!(lite_sql("a = $1 AND b = $12"), "a = ?1 AND b = ?12");
        assert_eq!(values_list(2, 2), "($1,$2),($3,$4)");
    }
}
