//! The archive database: schema and connection pool.
//!
//! Two stores:
//!
//! - the archive database (apakabar.db): messages, the search index and the analysis. It is
//!   rebuilt from the harvested text by `ingest` and `analyze` and can be thrown away.
//! - the site store (site.db next to it, or PostgreSQL when DATABASE_URL is set; see
//!   `site.rs`): accounts, bookmarks, editors' decisions, precision checks. Nothing in it
//!   can be rebuilt, so `ingest` never touches it.

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use std::path::Path;

pub type DbPool = Pool<SqliteConnectionManager>;

pub const SCHEMA_VERSION: &str = "4";

pub const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;

CREATE TABLE IF NOT EXISTS messages (
  id            INTEGER PRIMARY KEY,
  ak_id         TEXT NOT NULL UNIQUE,   -- permanent ID: AK-YYYY-MM-DD-NNNN, AK-YYYY-MM-NNNN, AKL-<archive>-NNNN
  collection    TEXT NOT NULL,          -- basisdata | list | iri-l (an unrelated list, kept apart)
  list_tag      TEXT,                   -- indonesia-l, indonesia-p ...
  date          TEXT,                   -- YYYY-MM-DD
  ym            TEXT,                   -- YYYY-MM
  year          INTEGER,
  date_source   TEXT,                   -- id | date_line | inner_date | header | neighbours | none
  date_note     TEXT,                   -- a caution about the date, when there is one
  subject       TEXT NOT NULL,          -- as posted
  title         TEXT NOT NULL,          -- the best title (see title_source)
  title_source  TEXT,                   -- subject | inner_subject | body_prefix | headline | opening | duplicate | topic
  topic         TEXT,                   -- conference topic, for replies filed under one ("(r)")
  is_topic_reply INTEGER DEFAULT 0,
  sender        TEXT,
  orig_from     TEXT,
  orig_date     TEXT,
  orig_subject  TEXT,
  source_code   TEXT,
  source_basis  TEXT,                   -- subject | inner_subject | url | text (where the source was read)
  source_name   TEXT,
  source_cat    TEXT,
  lang          TEXT,
  msg_type      TEXT,
  is_reply      INTEGER DEFAULT 0,
  dup_of        INTEGER,                -- canonical copy, when this is a duplicate
  counted       INTEGER DEFAULT 1,      -- 1 = counts in the figures (not a duplicate, not iri-l)
  thread_id     TEXT,                   -- AK ID of the thread's first message
  chars         INTEGER,
  words         INTEGER,
  headers       TEXT,
  body          TEXT NOT NULL,          -- cleaned, readable text
  raw_z         BLOB,                   -- raw text as recovered, zstd-compressed
  orig_url      TEXT,
  wayback_url   TEXT,
  repaired      INTEGER DEFAULT 0,
  html_fixed    INTEGER DEFAULT 0,
  sv_flag       INTEGER DEFAULT 0,      -- mentions rape or sexual violence
  may98_tier    INTEGER DEFAULT 0,      -- 1 mention, 2 same paragraph as the May riots, 3 focused (from 13 May 1998)
  may98_flag    INTEGER DEFAULT 0,      -- tier 2 or 3
  sensitive     INTEGER DEFAULT 0,      -- set at start-up from the site store: researchers only
  file          TEXT
);
CREATE INDEX IF NOT EXISTS ix_msg_date ON messages(date);
CREATE INDEX IF NOT EXISTS ix_msg_ym ON messages(ym);
CREATE INDEX IF NOT EXISTS ix_msg_source ON messages(source_code);
CREATE INDEX IF NOT EXISTS ix_msg_thread ON messages(thread_id);
CREATE INDEX IF NOT EXISTS ix_msg_dup ON messages(dup_of);
CREATE INDEX IF NOT EXISTS ix_msg_topic ON messages(topic);
CREATE INDEX IF NOT EXISTS ix_msg_counted ON messages(counted, date);

-- full-text search over the clean text. External content: the text is not stored twice.
-- The third column indexes Indonesian root words (diperkosa, pemerkosaan -> perkosa); the view
-- returns it empty because stems are only needed for matching, never for display.
CREATE VIEW IF NOT EXISTS fts_content AS SELECT id, title, body, '' AS stems FROM messages;
CREATE VIRTUAL TABLE IF NOT EXISTS fts USING fts5(title, body, stems, content='fts_content', content_rowid='id', tokenize='unicode61 remove_diacritics 2');

CREATE TABLE IF NOT EXISTS themes (id INTEGER PRIMARY KEY, slug TEXT UNIQUE, name TEXT, description TEXT);
-- every theme a message mentions; about = 1 when the model judges the theme a real subject of the text
CREATE TABLE IF NOT EXISTS message_themes (msg_id INTEGER, theme_id INTEGER, hits INTEGER, score REAL, about INTEGER DEFAULT 0, PRIMARY KEY(msg_id, theme_id)) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS ix_mt_theme ON message_themes(theme_id, msg_id);

CREATE TABLE IF NOT EXISTS entities (id INTEGER PRIMARY KEY, slug TEXT UNIQUE, name TEXT, kind TEXT, note TEXT, n_msgs INTEGER DEFAULT 0);
CREATE TABLE IF NOT EXISTS message_entities (msg_id INTEGER, entity_id INTEGER, hits INTEGER, PRIMARY KEY(msg_id, entity_id)) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS ix_me_entity ON message_entities(entity_id, msg_id);

CREATE TABLE IF NOT EXISTS events (id INTEGER PRIMARY KEY, slug TEXT UNIQUE, date TEXT, name TEXT, description TEXT, query TEXT);

CREATE TABLE IF NOT EXISTS analysis (key TEXT PRIMARY KEY, json TEXT NOT NULL, computed_at TEXT);

-- the database's own folder listings, captured by the Wayback Machine (see listings.rs)
CREATE TABLE IF NOT EXISTS day_listings (day TEXT PRIMARY KEY, listed INTEGER, src TEXT, ts TEXT) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS month_folders (ym TEXT PRIMARY KEY, days TEXT, src TEXT, ts TEXT) WITHOUT ROWID;
-- messages a listing names but no capture holds: date, number, size and the start of the title;
-- maybe = list-archive messages of the same days whose subject begins the same way
CREATE TABLE IF NOT EXISTS lost (ak_id TEXT PRIMARY KEY, date TEXT, num INTEGER, size TEXT, title TEXT, src TEXT, ts TEXT, maybe TEXT) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS ix_lost_date ON lost(date);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);
"#;

/// The indexes the server's start-up relies on. The two flags sit after the long text in each
/// row, so without them finding the May 1998 messages and the restricted ones reads the whole
/// table (about 1.4 GB): a second on a fast disk, a minute or more on a small host's. They are
/// partial: they hold only the few hundred flagged messages, so no query about the many others
/// (the catalogue's and the reading room's "not restricted") is tempted to use them.
pub const START_INDEXES: &str = "
CREATE INDEX IF NOT EXISTS ix_msg_may98 ON messages(may98_tier, ak_id) WHERE may98_tier >= 2;
CREATE INDEX IF NOT EXISTS ix_msg_restricted ON messages(sensitive, ak_id) WHERE sensitive = 1;
";

pub fn open_rw(path: &Path) -> rusqlite::Result<Connection> {
    let c = Connection::open(path)?;
    c.execute_batch(SCHEMA)?;
    c.execute_batch(START_INDEXES)?;
    c.execute_batch("PRAGMA cache_size = -200000; PRAGMA temp_store = MEMORY;")?;
    Ok(c)
}

/// The messages readers may not open, as a subquery the restricted-messages index answers
/// alone. Lists and searches test `m.id NOT IN` this rather than reading `sensitive` from each
/// row: the flag sits after the text, so reading it means reading every message whole (360 MB
/// for a title search over "timor", against half a megabyte this way).
pub const RESTRICTED_IDS: &str = "(SELECT id FROM messages WHERE sensitive = 1)";

/// Add the start-up indexes to a database built before they existed. Returns true when it
/// had to (that reads the table once).
pub fn ensure_start_indexes(path: &Path) -> rusqlite::Result<bool> {
    let c = Connection::open(path)?;
    c.busy_timeout(std::time::Duration::from_secs(30))?;
    let have: i64 = c.query_row("SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name IN ('ix_msg_may98', 'ix_msg_restricted')", [], |r| r.get(0))?;
    if have == 2 {
        return Ok(false);
    }
    c.execute_batch(START_INDEXES)?;
    Ok(true)
}

/// Mark the messages readers may not open: those the editors restricted and those still
/// waiting for review. Researchers and editors see them all. The flags live in the site
/// store; this copies them into the archive database, where listings and search filter on them.
/// Only the messages whose flag changes are written, so a restart usually writes nothing.
pub fn apply_restricted(archive: &Connection, restricted: &std::collections::HashSet<String>) -> rusqlite::Result<usize> {
    let marked: std::collections::HashSet<String> = archive.prepare("SELECT ak_id FROM messages WHERE sensitive = 1")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let lift: Vec<&String> = marked.difference(restricted).collect();
    let add: Vec<&String> = restricted.difference(&marked).collect();
    if !lift.is_empty() || !add.is_empty() {
        let tx = archive.unchecked_transaction()?;
        {
            let mut off = tx.prepare("UPDATE messages SET sensitive = 0 WHERE ak_id = ?1")?;
            for a in &lift {
                off.execute([a])?;
            }
            // an AK ID the archive does not hold (any more) matches nothing
            let mut on = tx.prepare("UPDATE messages SET sensitive = 1 WHERE ak_id = ?1")?;
            for a in &add {
                on.execute([a])?;
            }
        }
        tx.commit()?;
    }
    archive.query_row("SELECT count(*) FROM messages WHERE sensitive = 1", [], |r| r.get::<_, i64>(0)).map(|n| n as usize)
}

/// Pool of connections to the archive database, sized for a small machine (2 GB of memory).
/// Reads go through a memory map of the file, so the pages live once in the system's file
/// cache (shared, and given back under pressure) and each connection keeps only a small cache
/// of its own: the server's own memory stays under 100 MB. (Tools that add up the memory map
/// of each connection show a much larger figure; it is the same file pages counted again.)
pub fn pool(path: &Path) -> DbPool {
    let mgr = SqliteConnectionManager::file(path).with_init(|c| {
        c.execute_batch("PRAGMA busy_timeout = 5000; PRAGMA cache_size = -16000; PRAGMA temp_store = MEMORY; PRAGMA mmap_size = 2147483648;")
    });
    Pool::builder().max_size(8).build(mgr).expect("open database")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn set(v: &[&str]) -> HashSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn restricted_flags_follow_the_store() {
        let dir = std::env::temp_dir().join(format!("apakabar-db-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.db");
        let _ = std::fs::remove_file(&p);
        {
            // a database built before the start-up indexes existed
            let c = Connection::open(&p).unwrap();
            c.execute_batch(SCHEMA).unwrap();
            for (k, a) in ["AK-1", "AK-2", "AK-3"].iter().enumerate() {
                c.execute("INSERT INTO messages(ak_id, collection, subject, title, body, may98_tier) VALUES (?1, 'basisdata', 's', 't', 'b', ?2)", rusqlite::params![a, k as i64 + 1]).unwrap();
            }
        }
        assert!(ensure_start_indexes(&p).unwrap());
        assert!(!ensure_start_indexes(&p).unwrap());
        let c = Connection::open(&p).unwrap();
        // the start-up queries read the indexes, not the table
        for (q, ix) in [("SELECT ak_id FROM messages WHERE may98_tier >= 2", "ix_msg_may98"), ("SELECT ak_id FROM messages WHERE sensitive = 1", "ix_msg_restricted"), ("SELECT count(*) FROM messages WHERE sensitive = 1", "ix_msg_restricted")] {
            let plan: String = c.query_row(&format!("EXPLAIN QUERY PLAN {q}"), [], |r| r.get(3)).unwrap();
            assert!(plan.contains(ix) && plan.contains("COVERING"), "{plan}");
        }
        for q in ["SELECT count(*) FROM messages WHERE sensitive = 0 AND counted = 1", "SELECT ak_id FROM messages WHERE may98_tier < 2"] {
            let plan: String = c.query_row(&format!("EXPLAIN QUERY PLAN {q}"), [], |r| r.get(3)).unwrap();
            assert!(!plan.contains("ix_msg_may98") && !plan.contains("ix_msg_restricted"), "{plan}");
        }
        assert_eq!(apply_restricted(&c, &set(&["AK-2", "AK-3", "AK-gone"])).unwrap(), 2);
        assert_eq!(apply_restricted(&c, &set(&["AK-3", "AK-1"])).unwrap(), 2);
        let on: HashSet<String> = c.prepare("SELECT ak_id FROM messages WHERE sensitive = 1").unwrap().query_map([], |r| r.get(0)).unwrap().map(|x| x.unwrap()).collect();
        assert_eq!(on, set(&["AK-1", "AK-3"]));
        assert_eq!(apply_restricted(&c, &set(&[])).unwrap(), 0);
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
