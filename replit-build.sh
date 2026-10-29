#!/bin/sh
# Build the Apakabar Archive on Replit (it works on any Linux or macOS machine too):
# the program, and the archive database from the texts in data/ when it is missing.
#
#   sh replit-build.sh            build whatever is missing (safe to run again at any time)
#   sh replit-build.sh --analyze  recompute the figures after a change to the program
#   sh replit-build.sh --rebuild  rebuild the archive database from the texts
#
# The first run takes 15 to 30 minutes. After that it only checks that everything is in place,
# which is what the build step of each publish does.
set -e
cd "$(dirname "$0")"
SCHEMA=4
MIN_RUST=88 # Rust 1.88 or newer
MODE="${1:-}"

say() { printf '\n== %s\n' "$*"; }
rust_minor() { rustc --version 2>/dev/null | sed -n 's/^rustc 1\.\([0-9]*\)\..*/\1/p'; }

# ---------------------------------------------------------------- the program
up_to_date() {
  [ -x target/release/apakabar ] && [ -z "$(find src Cargo.toml Cargo.lock build.rs -newer target/release/apakabar 2>/dev/null | head -n 1)" ]
}
if up_to_date; then
  say "The program is up to date."
else
  # a Rust installed by an earlier run of this script
  if [ -x .toolchain/cargo/bin/cargo ]; then
    export CARGO_HOME="$PWD/.toolchain/cargo" RUSTUP_HOME="$PWD/.toolchain/rustup"
    export PATH="$CARGO_HOME/bin:$PATH"
  fi
  m=$(rust_minor)
  if [ -z "$m" ] || [ "$m" -lt "$MIN_RUST" ]; then
    say "Rust 1.$MIN_RUST or newer is needed; installing the current Rust into .toolchain/ (official installer, about 2 minutes)"
    command -v curl >/dev/null 2>&1 || { echo "curl is not available: add Rust to this app instead (DEPLOY-REPLIT.md, Problems)."; exit 1; }
    export CARGO_HOME="$PWD/.toolchain/cargo" RUSTUP_HOME="$PWD/.toolchain/rustup"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path
    export PATH="$CARGO_HOME/bin:$PATH"
  fi
  if ! command -v cc >/dev/null 2>&1 && ! command -v gcc >/dev/null 2>&1 && ! command -v clang >/dev/null 2>&1; then
    echo "No C compiler was found. In Replit, open Tools > Dependencies > System, add gcc, and run this script again."
    exit 1
  fi
  say "Building the program with $(rustc --version) (3 to 8 minutes the first time)"
  cargo build --release --locked
fi

# The system libraries the program runs on, kept beside it: a host whose system lacks the
# standard program loader (as a published Replit app may) runs the program through these.
if [ -e /lib64/ld-linux-x86-64.so.2 ] && [ ! -e runtime/ld-linux-x86-64.so.2 ]; then
  mkdir -p runtime
  for f in /lib64/ld-linux-x86-64.so.2 /lib/x86_64-linux-gnu/libc.so.6 /lib/x86_64-linux-gnu/libm.so.6 /lib/x86_64-linux-gnu/libgcc_s.so.1; do
    if [ -e "$f" ]; then cp -L "$f" runtime/; fi
  done
fi

# ---------------------------------------------------------------- the archive database
if [ "$MODE" = "--analyze" ] && [ -f apakabar.db ]; then
  say "Recomputing the figures"
  ./target/release/apakabar analyze --db apakabar.db
  exit 0
fi
if [ "$MODE" != "--rebuild" ] && [ -f apakabar.db ] && [ "$(cat .schema 2>/dev/null)" = "$SCHEMA" ]; then
  say "The archive database is up to date. Press Run, or publish."
  exit 0
fi

if [ ! -d "data/01 Apakabar Messages (1990-2002)" ]; then
  if [ ! -f data/apakabar-data.tar.gz.part-00 ]; then
    echo "The texts are missing: upload apakabar-data.tar.gz.part-00 to part-30 and SHA256SUMS.txt into the data folder (DEPLOY-REPLIT.md, step 3)."
    exit 1
  fi
  if command -v sha256sum >/dev/null 2>&1 && [ -f data/SHA256SUMS.txt ]; then
    say "Checking the uploaded files"
    (cd data && sha256sum -c SHA256SUMS.txt) || { echo "One of the uploaded files is incomplete or damaged: upload it again."; exit 1; }
  fi
  say "Unpacking the texts"
  cat data/apakabar-data.tar.gz.part-* | tar -xzf - -C data
fi

say "Building the archive database (10 to 20 minutes; the result is about 1.8 GB)"
./target/release/apakabar ingest --input data --db apakabar.db
./target/release/apakabar analyze --db apakabar.db
echo "$SCHEMA" > .schema

say "Removing the unpacked texts (the uploaded files stay in data/)"
rm -rf "data/01 Apakabar Messages (1990-2002)" "data/02 Apakabar Lists Hypermail Archives (indopubs.com, 1997-2002)" "data/00 Index and Working Files"
say "Done. Press Run to try the site, or publish it."
