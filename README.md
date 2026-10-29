# Apakabar Archive: website

A single Rust program that builds, analyses and serves the archive of John A. MacDougall's
apakabar lists (October 1990 – February 2002), in English and Indonesian. The site has two
parts:

- **Public pages**: the analytical essay (homepage), data pages for themes, sources, events, people
  and words, two case studies (May 1998 and Gerwani/1965), a catalogue of titles, and an About page
  with the terms of use and the contact for corrections and takedown requests.
- **Reading room** (login, approved by an editor): full-text search in Indonesian and English,
  browsing by month, day, source, theme, person, event, conference topic and collection, a clean
  reading view with the raw text one click away, citations, bookmarks with notes, CSV export, and
  the list of messages the database held that no capture kept (`/read/lost`).

Every page has an English and an Indonesian version (the switch is at the top right; the choice
is remembered). The editors' pages are in English. There is no JavaScript framework, no web fonts
and no external service. Pages are rendered on the server with inline SVG charts and take a few
milliseconds each.

To put the site online with Replit, follow **DEPLOY-REPLIT.md**.

## Quick start on your own computer (macOS or Linux)

`./run.sh` builds the program, builds the archive database when it is missing or was made by an
older version, and starts the site at http://127.0.0.1:8080. The first time it prints a link to
create your editor account:

```
There is no editor account yet. Create yours here (the link works once, until the server restarts):
  http://127.0.0.1:8080/setup?token=…
```

By hand:

1. Install Rust 1.85 or newer, once: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
2. Build, from this folder: `cargo build --release` (a few minutes the first time).
3. Build the database from the harvested text files (about 3 minutes; the result is about 1.8 GB):
   ```
   ./target/release/apakabar ingest --input ".." --db apakabar.db
   ./target/release/apakabar analyze --db apakabar.db
   ```
   Here `--input` is the "Apakabar Indonesia Archive" folder. The program reads
   "01 Apakabar Messages (1990-2002)", "02 Apakabar Lists Hypermail Archives …" and the
   database's folder listings in "00 Index and Working Files/_cache/listings" (see Coverage below).
4. Run the site: `./target/release/apakabar serve --db apakabar.db`, open the setup link it prints,
   and create your editor account. (Or make accounts from the command line:
   `./target/release/apakabar user --db apakabar.db --email you@example.org --name "Your Name" --role admin`.)

## Two stores

- **apakabar.db** holds the messages, the search index and the analysis. `ingest` rebuilds it from
  the text files (into `apakabar.db.building`, renamed into place when complete) and it can always
  be thrown away. Schema version 4; the server refuses an older file and asks for `ingest`.
- **The site store** holds what cannot be rebuilt: accounts, sessions, bookmarks and notes, the
  privacy review (decisions and the first-pass suggestions), restricted messages, the
  precision checks, password-reset links and the audit log. Everything in it refers to messages by
  their permanent AK ID, so a rebuilt archive keeps it all.
  - When the environment variable `DATABASE_URL` holds a PostgreSQL address (as on Replit), the
    store is that database. The connection is encrypted when the server offers it, and the
    server's certificate must check out against the public certificate authorities.
  - Otherwise it is **site.db**, a SQLite file next to apakabar.db (or `--site FILE`).

The first-pass files in `tools/` are loaded into the store automatically (once per version of each
file).

## Commands

| Command | What it does |
|---|---|
| `ingest --input DIR --db FILE` | Parses, cleans, dates, titles, de-duplicates, threads, classifies and indexes every message |
| `analyze --db FILE [--site FILE]` | Tags themes and names, scores the May 1998 flag and computes every figure on the site (stored in the `analysis` table) |
| `serve --db FILE [options]` | Runs the website (options below) |
| `user --db FILE --email … --role reader/researcher/admin` | Creates or updates an account (in PostgreSQL when `DATABASE_URL` is set) |
| `export --db FILE --out DIR` | Writes the catalogue in the Gerwani Archive index format: two workbooks (README, Items, coverage, lost messages, themes, precision, May 1998, outlets, data dictionary) and a gzip-compressed CSV of every row |
| `features --db FILE --ids FILE --out FILE` | (for maintainers, hidden) the model features of the listed messages, for refitting the theme model |

Options of `serve` (each can also be given as an environment variable):

| Option | Environment variable | Meaning |
|---|---|---|
| `--addr HOST:PORT` | `APAKABAR_ADDR` | where to listen (default 127.0.0.1:8080) |
| `--contact ADDRESS` | `APAKABAR_CONTACT` | the address for corrections and takedown requests on the About page |
| `--base-url URL` | `APAKABAR_BASE_URL` | the site's public address, used in setup and reset links |
| `--content DIR` | `APAKABAR_CONTENT` | the folder of the essay files (default `content`) |
| `--secure-cookies` | | cookies only over HTTPS, and HSTS (use behind HTTPS) |
| `--trust-proxy` | | read the client's address and host from the proxy's X-Forwarded headers |
| `--require-database` | `APAKABAR_REQUIRE_DATABASE` | refuse to start without `DATABASE_URL` (for hosts whose files do not last) |
| `--open-registration` | | let new readers in without an editor's approval (by default an editor approves each) |
| `--admin-reset EMAIL` | `APAKABAR_ADMIN_RESET` | print a one-time link to set a new password for that editor (for an editor who is locked out) |

Roles:

- **reader**: the reading room, except restricted messages; e-mail addresses in the texts are
  masked (`•••@domain`), list addresses excepted.
- **researcher**: also opens restricted messages and sees addresses unmasked.
- **admin**: an editor. Also gets `/admin` (approving readers, roles, reset links, restricted
  messages, audit log), `/admin/review` (the privacy review) and `/admin/checks` (precision checks).

## Privacy

- Messages that speak of the sexual violence of May 1998 (tiers 2 and 3 of the May 1998 flag, 638
  messages with copies) are hidden from readers until an editor has reviewed them on
  `/admin/review`: *clear* opens a message to readers, *restrict* keeps it to researchers. A
  decision covers every copy of the text. Their titles are withheld on the public pages (essay
  lists, catalogue, case study) until they are cleared.
- `tools/first_pass_review.json` holds a first-pass suggestion for each of the 620 texts
  (restrict 22, of them 6 marked "Unsure"; show 598), made in October 2026 from excerpts: restrict
  when the text names or identifies a survivor or a private person caught up in the violence
  (name, or initials or a pseudonym together with age, street, school, workplace or family),
  show when it reports, argues or denies without identifying anyone. They are suggestions only;
  the editor decides.
- Editors can restrict any other message from its page.

## Languages and the essay

- The essay on the homepage is in `content/home.en.md` and `content/home.id.md`: Markdown with a
  little HTML. Figures are written as `{{name}}` and filled in from the analysis; charts as blocks
  on a line of their own (`{{figure:monthly}}` …). The comment at the top of each file lists them.
  The server reads the files when it starts (restart after a change); if a file is missing it
  uses the copy built into the program.
- Every other text of the interface is in the program, in both languages (`t("English",
  "Indonesia")` in `src/web/`, theme and event names in `src/web/i18n.rs`). Numbers and dates are
  written the Indonesian way on Indonesian pages (137.146; 14 Mei 1998; 99,5%).

## Themes: mentions and "about"

Each of the 19 themes is a published list of terms, weak words (counted only beside a term),
context words and ignored phrases (`src/lexicon.rs`). A message *mentions* a theme when the words
pass the rules in `src/tagger.rs`. Many such messages only touch on the theme, so a logistic model
(`src/model.rs`) reads how the theme's words fall in the message (how many, how early, how
concentrated, in how many paragraphs) and decides whether it is *about* the theme. The site's
figures count the messages about a theme; the reading room can list every mention.

The model was fitted on 914 hand-checked messages. After editing the lexicon, refit it:

```
./target/release/apakabar features --db apakabar.db --ids checked_ids.txt --out features.jsonl
python3 tools/fit_model.py --features features.jsonl --verdicts verdicts.jsonl \
   --themes "$(grep -o 'Theme { slug: "[a-z0-9-]*"' src/lexicon.rs | sed 's/.*"\(.*\)"/\1/' | paste -sd, -)" \
   --out src/model.rs
cargo build --release && ./target/release/apakabar analyze --db apakabar.db
```

The May 1998 flag (`src/flags.rs`) has three tiers from 13 May 1998: 1 mentions rape or sexual
violence (figures of speech left out), 2 speaks of the sexual violence of the May riots or the
inquiries into it, 3 is mainly about it.

## Precision checks

Every theme, flag, name list and kind of derived title has a check set: 50 messages drawn at
random (the draw is kept in the site store, so a set is always read on the same messages).
Editors answer one question per message on `/admin/checks`; the site quotes the latest reading of
each set beside its figures and on that page. A set checked again after a change gets a new key
("/2", "/3") and a fresh sample.

`tools/first_pass_checks.json` holds the first-pass reading made in October 2026 (3,350
verdicts in three rounds); it is loaded automatically. Editors' own verdicts replace the first
pass on the site as they are entered. Latest first-pass results: all 19 themes at 90% or more
except the May 1998 riots theme (44 of 49); May 1998 tier 2 98%, tier 3 86%; names 94–98%; titles
completed from the text or from another copy 100%, from a headline line 82%, from the opening
words 46% (these are marked as such wherever they appear).

## Coverage and lost messages

The basisdata kept each day's messages as numbered files in a folder per day, and the servers that
hosted it (Hamline University, Ohio University Library, the esosoft mirror) showed every folder as
an Apache "Index of" page: file number, size and the first 23 characters of the title. The Wayback
Machine kept those pages for 3,746 days, and month pages that list the day folders. They were read
in October 2026 into `00 Index and Working Files/_cache/listings/*.json`; `ingest` loads them
(`src/listings.rs`) into the tables `day_listings`, `month_folders` and `lost`.

With them `analyze` counts the coverage instead of estimating it: the database held 137,146
messages, and 136,431 (99.5%) are recovered. On 334 days nothing was filed at all (the month page
shows no folder for the day, as for 14 May 1998). The 715 messages a listing names but no capture
holds are on `/read/lost` and in the index sheet "Lost messages", each with its listed size, the
start of its title and any list-archive message from a day before to two days after whose title
begins the same way (a possible copy). Without the listings folder the program falls back to the
highest message number of each day and gives an empty day the median of the weeks around it, which
counts days on which nothing was filed as losses.

## Putting it online

- **Replit** (chosen for apakabar.nathpribady.com): see `DEPLOY-REPLIT.md`. `.replit`,
  `replit-build.sh` and `replit-start.sh` are the configuration; `data/` holds the texts as 31
  compressed parts for upload (`SHA256SUMS.txt` checks them).
- **Any Linux server** (1 vCPU, 2 GB memory, 5 GB disk): build there, run `ingest` and `analyze`,
  and run `apakabar serve --addr 127.0.0.1:8080 --secure-cookies --contact … --base-url https://…`
  under systemd behind Caddy or nginx for HTTPS (`apakabar.example.org { reverse_proxy
  127.0.0.1:8080 }`; add `--trust-proxy`). While serving it uses about 40 MB of memory.

## What the program does (source map)

| File | Role |
|---|---|
| `src/parse.rs` | Reads the harvested monthly files into records; IDs and their dates (day folders, the month folder of early 2000, list archives) |
| `src/clean.rs` | Hypermail text → readable paragraphs; HTML leftovers and entities; nested mail and conference headers moved aside; lost apostrophes; quoted-printable |
| `src/titles.rs` | The best title: the subject, a subject cut short completed from the text, the inner subject of a forward, a headline line, or the opening words (`title_source` says which) |
| `src/sources.rs` | Outlet codes (KMP, JKTP, SIAR …), about 150 in 13 categories; subject parsing; IGC conference topics; citations in opening lines |
| `src/text.rs` | Tokens, stop words, language identification, and the Indonesian root-word stemmer (Sastrawi dictionary: diperkosa, pemerkosaan → perkosa) |
| `src/ingest.rs` | Parallel pipeline: dates, titles, duplicates (exact and 64-bit SimHash), threads, full-text index (SQLite FTS5 with a root-word column), the folder listings and the lost messages |
| `src/listings.rs` | Reads the database's own folder listings (Wayback captures of the "Index of" pages at Hamline, Ohio and esosoft): the messages each day held, the days without a folder, the AK IDs no capture holds |
| `src/lexicon.rs` | The 19 themes, 117 names and 35 dated events. **Edit these lists, refit the model if themes changed, and re-run `analyze`** |
| `src/tagger.rs` | Matches themes and names (whole words; datelines, credits, addresses and set phrases left out) and computes the model's features |
| `src/model.rs` | The theme model's weights and per-theme thresholds (generated by `tools/fit_model.py`) |
| `src/flags.rs` | The sexual-violence and May 1998 flag |
| `src/analyze.rs` | Every statistic on the site: coverage by day (exact where a listing survives), monthly volumes, languages, outlets, the underground press, theme shares, names and co-occurrence, event attention, distinctive words per year (Dunning's G²), the May 1998 and 1965 case studies, check sets |
| `src/site.rs` | The site store on SQLite or PostgreSQL (accounts, sessions, bookmarks, review, checks, reset links, audit) and the loading of the first-pass files |
| `src/export.rs` | The Gerwani-format index |
| `build.rs` | On Linux x86-64, makes the program ask for the system's standard loader, so a program built on Replit also runs once published |
| `src/web/` | Axum server: `public.rs` (essay and data pages, login, registration), `read.rs` (reading room, lost messages), `account.rs` (password change, reset links, the setup page), `auth.rs` (Argon2 passwords, sessions, roles), `admin.rs` (editors, precision checks), `review.rs` (privacy review), `security.rs` (form tokens, origin check, login limits, headers), `i18n.rs` (languages, numbers and dates), `charts.rs` (SVG), `layout.rs` (HTML shell and CSS) |
| `content/` | The homepage essay in English and Indonesian |
| `tools/` | `fit_model.py` (refits the theme model), `first_pass_checks.json` (first-pass precision checks), `first_pass_review.json` (first-pass privacy suggestions); the two first-pass files stay out of the public repository, as they point to the messages that identify survivors |

## Protection of the texts

- Full texts are visible only after login, and new accounts wait for an editor's approval.
  `robots.txt` keeps search engines out of the reading room. Each account can open 1,500 messages
  a day, to discourage bulk copying. A login lowers the risk of copyright complaints but is not a
  legal safe harbour: answer takedown requests promptly.
- Passwords (10 characters or more) are hashed with Argon2id. Sessions are random tokens in
  HttpOnly, SameSite cookies (Secure behind HTTPS), kept for 30 days, and the store holds only
  their SHA-256 hashes; reset links likewise. Changing or resetting a password ends the other
  sessions; blocking an account ends all of them.
- Every form carries a token tied to the browser (double-submit cookie) and posts from other sites
  are refused. Failed logins are limited to 10 per address and 5 per account in 15 minutes;
  registrations to 5 per address an hour.
- Pages are sent with a strict Content-Security-Policy (scripts, styles and forms only from the
  site itself, which uses no scripts at all), X-Frame-Options DENY,
  nosniff, a same-origin referrer policy, HSTS behind HTTPS, and `Cache-Control: no-store` on the
  reading room and the editors' pages.
- Readers see e-mail addresses masked; restricted messages (see Privacy) open only for researchers
  and editors.
