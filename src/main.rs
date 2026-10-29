//! Apakabar Archive: one small binary that builds, analyses and serves the archive of
//! John A. MacDougall's apakabar lists (1990-2002).
//!
//!   apakabar ingest  --input "<Apakabar Indonesia Archive folder>" --db apakabar.db
//!   apakabar analyze --db apakabar.db
//!   apakabar serve   --db apakabar.db --addr 127.0.0.1:8080
//!   apakabar user    --db apakabar.db --email you@example.org --name "You" --role admin
//!   apakabar export  --db apakabar.db --out "<folder>"
//!
//! Accounts, bookmarks and editors' decisions are kept in the site store: PostgreSQL when
//! DATABASE_URL is set (hosts without a lasting disk, such as Replit), otherwise site.db next
//! to apakabar.db (or the file given with --site). Rebuilding the archive never touches them.

mod analyze;
mod clean;
mod db;
mod export;
mod flags;
mod ingest;
mod lexicon;
mod listings;
mod model;
mod parse;
mod site;
mod sources;
mod tagger;
mod text;
mod titles;
mod web;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "apakabar", about = "Apakabar Archive: ingest, analyse and serve the apakabar lists (1990-2002)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build the archive database from the harvested monthly text files
    Ingest {
        #[arg(long)]
        input: PathBuf,
        #[arg(long, default_value = "apakabar.db")]
        db: PathBuf,
    },
    /// Tag themes and names, compute every statistic shown on the site
    Analyze {
        #[arg(long, default_value = "apakabar.db")]
        db: PathBuf,
        /// site database (default: site.db next to the archive database)
        #[arg(long)]
        site: Option<PathBuf>,
    },
    /// Run the website
    Serve {
        #[arg(long, default_value = "apakabar.db")]
        db: PathBuf,
        /// site database file (ignored when DATABASE_URL is set)
        #[arg(long)]
        site: Option<PathBuf>,
        #[arg(long, default_value = "127.0.0.1:8080", env = "APAKABAR_ADDR")]
        addr: String,
        /// let new readers in without an editor's approval (by default an editor approves each)
        #[arg(long)]
        open_registration: bool,
        /// (kept for old scripts: approval is now the default)
        #[arg(long, hide = true)]
        approval: bool,
        /// set the Secure flag on cookies and send HSTS (use behind HTTPS)
        #[arg(long)]
        secure_cookies: bool,
        /// trust X-Forwarded-For and X-Forwarded-Host from the hosting proxy (Replit, nginx)
        #[arg(long)]
        trust_proxy: bool,
        /// the public address of the site, e.g. https://apakabar.nathpribady.com
        #[arg(long, env = "APAKABAR_BASE_URL")]
        base_url: Option<String>,
        /// address for corrections and takedown requests, shown on the About page
        #[arg(long, default_value = "", env = "APAKABAR_CONTACT")]
        contact: String,
        /// folder of the editable texts (home.en.md, home.id.md)
        #[arg(long, default_value = "content", env = "APAKABAR_CONTENT")]
        content: PathBuf,
        /// refuse to start without DATABASE_URL (on hosts whose files do not last, such as a
        /// Replit deployment, accounts kept in a file would be lost at the next deploy)
        #[arg(long, env = "APAKABAR_REQUIRE_DATABASE")]
        require_database: bool,
        /// print a one-time link to set a new password for this editor account at start (for an
        /// editor who cannot log in; remove the setting afterwards)
        #[arg(long, env = "APAKABAR_ADMIN_RESET")]
        admin_reset: Option<String>,
    },
    /// Create or update a reading-room account (prompts for a password if not given)
    User {
        #[arg(long, default_value = "apakabar.db")]
        db: PathBuf,
        #[arg(long)]
        site: Option<PathBuf>,
        #[arg(long)]
        email: String,
        #[arg(long, default_value = "")]
        name: String,
        #[arg(long, default_value = "reader")]
        role: String,
        #[arg(long)]
        password: Option<String>,
    },
    /// (for maintainers) write the model features of the listed messages, one JSON line per theme
    #[command(hide = true)]
    Features {
        #[arg(long, default_value = "apakabar.db")]
        db: PathBuf,
        #[arg(long)]
        ids: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Write the catalogue in the Gerwani Archive index format (xlsx and csv)
    Export {
        #[arg(long, default_value = "apakabar.db")]
        db: PathBuf,
        /// folder for the index files
        #[arg(long)]
        out: PathBuf,
        /// path of the archive folder as recorded in file_path_in_parent
        #[arg(long, default_value = "Downloads/Apakabar Indonesia Archive")]
        parent: String,
    },
}

fn main() {
    let cli = Cli::parse();
    let res = match cli.cmd {
        Cmd::Ingest { input, db } => ingest::run(&input, &db),
        Cmd::Analyze { db, site } => analyze::run(&db, site.as_deref()),
        Cmd::Serve { db, site, addr, open_registration, approval, secure_cookies, trust_proxy, base_url, contact, content, require_database, admin_reset } => {
            let _ = approval;
            web::serve(web::ServeOpts { db, site, addr, approval: !open_registration, secure_cookies, trust_proxy, base_url, contact, content, require_database, admin_reset })
        }
        Cmd::User { db, site, email, name, role, password } => web::auth::cli_user(&db, site.as_deref(), &email, &name, &role, password),
        Cmd::Export { db, out, parent } => export::run(&db, &out, &parent),
        Cmd::Features { db, ids, out } => analyze::dump_features(&db, &ids, &out),
    };
    if let Err(e) = res {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
