#!/bin/sh
# Build the Apakabar Archive website and run it on http://127.0.0.1:8080
# Usage: ./run.sh   (from the "03 Website (Rust)" folder inside "Apakabar Indonesia Archive")
#
# The archive database (apakabar.db) is built from the text folders next to this one when it is
# missing or was made by an older version. Accounts, bookmarks and editors' decisions live in
# site.db (or in PostgreSQL when DATABASE_URL is set), which a rebuild never touches. The first
# time, the server prints a link to create your editor account.
set -e
cd "$(dirname "$0")"
command -v cargo >/dev/null || { echo "Install Rust first: https://rustup.rs"; exit 1; }
cargo build --release
SCHEMA=4
if [ ! -f apakabar.db ] || [ "$(cat .schema 2>/dev/null)" != "$SCHEMA" ]; then
  ./target/release/apakabar ingest --input ".." --db apakabar.db
  ./target/release/apakabar analyze --db apakabar.db
  echo "$SCHEMA" > .schema
fi
exec ./target/release/apakabar serve --db apakabar.db --addr 127.0.0.1:8080 --contact nath@logosid.app
