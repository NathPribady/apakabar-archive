# Putting the Apakabar Archive online with Replit

This guide publishes the site at **https://apakabar.nathpribady.com** on a Replit Reserved VM
(0.5 vCPU, 2 GB memory, $15 a month in October 2026). It takes about an hour, most of it
waiting for builds.

What ends up where:

| What | Where it lives | Lasts |
|---|---|---|
| The program, the essay files, the archive database (apakabar.db, built from the texts) | the Replit app's files, copied into each publish | rebuilt from the uploaded texts whenever needed |
| Accounts, sessions, bookmarks, privacy decisions, precision checks, audit log | the app's PostgreSQL database (Replit's production database) | permanent; not touched by a publish or a rebuild |

The texts themselves are in the app's files, so **the Replit app must stay private** (see the
end of this guide).

## 1. Create the app

1. Sign in to Replit with the plan that pays for the Reserved VM (Core or higher).
2. Create a new app: **Import → Empty → Create empty project** (it starts without the Replit
   Agent), then rename it `apakabar-archive` (project menu → Edit project details). Do not ask
   the Replit Agent to build anything.
3. Check that the app is **private**: new apps on the paid plans are, and opened while logged
   out, the app's address (replit.com/@you/…) shows "not found".

## 2. Upload the program

From the folder **03 Website (Rust)** on your computer, upload into the app's Files pane:

- the folders `src`, `content`, `tools` and `data` (the data files go in step 3),
- the files `Cargo.toml`, `Cargo.lock`, `replit-build.sh`, `replit-start.sh`, `README.md`,
  this guide, and the hidden file **`.replit`**.

Drag them onto the Files pane, or use its menu (⋮) → *Upload folder* / *Upload file*. On a
Mac, Finder hides `.replit`: press **Cmd + Shift + .** in the Finder window to show it. If
Replit already made a `.replit` and a `src/main.rs`, let the uploaded ones replace them, and
delete any other file the template made (for example a second `main.rs` outside `src`).

## 3. Upload the texts

Upload the 32 files in **03 Website (Rust)/data** into the app's `data` folder: the 31 parts
`apakabar-data.tar.gz.part-00` to `part-30` (9.7 MB each, 295 MB in all; small parts upload
more reliably from a browser) and `SHA256SUMS.txt`.

They hold the harvested monthly files and the folder listings, compressed (the build checks
that nothing was damaged on the way).

## 4. Add the database

Open **Tools → Database** and create a PostgreSQL database. Replit then gives the app a
`DATABASE_URL` secret by itself; you do not need to copy anything.

## 5. Build

Open the **Shell** tab and run:

```
sh replit-build.sh
```

The first run takes 15 to 30 minutes. It installs a recent Rust if the one on Replit is too
old, compiles the program, unpacks the texts, builds the archive database (about 1.8 GB) and
computes every figure. It ends with `== Done. Press Run to try the site, or publish it.` If it
stops with a message, see *Problems* below; running it again resumes where it stopped.

## 6. Try it in the workspace

Press **Run**. The Console prints, among other lines:

```
site store: PostgreSQL (DATABASE_URL)
638 messages wait for an editor's review; 0 are restricted to researchers
There is no editor account yet. Create yours here (the link works once, until the server restarts):
  https://….replit.dev/setup?token=…
```

Open the site in a **new browser tab** (the address in the Webview's address bar, or the
link above). The Webview pane inside Replit stays blank: the site refuses to be shown inside
another page, which also protects it from click-jacking.

This first run also writes the site's tables into the development database, which Replit needs
before it can create the production database in the next step. Accounts you make here belong
to the development database and do not carry over to the published site. Stop the Run when you
have looked around.

## 7. Publish

Open **Publish** (or **Deploy**) and choose:

- **Reserved VM**, machine **0.5 vCPU / 2 GB** ($15 a month), type **Web server**;
- Build command `sh replit-build.sh`, Run command `sh replit-start.sh` (both are filled in from
  `.replit`);
- Port: 8080 inside, 80 outside (also from `.replit`);
- Production database: on. Replit creates it at the first publish; in older apps the setting
  is called *Create production database*. Do not copy development data into it.

Publish. The build step only checks that everything is in place (a minute or two), then the site
starts. The published site keeps accounts in the production database and refuses to start
without one, so that no account can be lost at a later publish.

## 8. Create your editor account

Open the deployment's **Logs**. Near the top:

```
There is no editor account yet. Create yours here (the link works once, until the server restarts):
  https://apakabar.nathpribady.com/setup?token=…
```

If the domain is not connected yet (step 9), use the same link with the app's `….replit.app`
address in place of `apakabar.nathpribady.com`. Fill in your name, `nath@logosid.app` and a
password of at least 10 characters. You arrive on the editors' page, `/admin`. The link stops
working as soon as the account exists.

## 9. Connect apakabar.nathpribady.com

1. In the deployment's settings (or **Tools → Domains**), add the domain
   `apakabar.nathpribady.com`.
2. Replit shows two records for it. Add both at the company that runs the DNS of
   nathpribady.com:
   - an **A** record, host/name `apakabar`, value the IP address Replit shows;
   - a **TXT** record, host/name `apakabar`, value `replit-verify=…` as shown.
3. If the DNS is at Cloudflare, set the A record to **DNS only** (grey cloud), not proxied.
   Remove any other A or AAAA record for `apakabar`.
4. Keep the TXT record for good: Replit needs it to renew the HTTPS certificate.

Replit checks every minute or two and shows **Verified** when the records are found (usually
within an hour; up to 48 hours). HTTPS is set up by Replit automatically.

## 10. Before you announce the site

- **Privacy review.** On `/admin/review`, 620 texts about the sexual violence of May 1998 wait
  for your decision (638 messages counting copies). They stay hidden from ordinary readers until
  you clear them. Beside each one is a first-pass suggestion: 22 to restrict (6 of them
  marked "Unsure"), 598 to show. The suggestions were made from excerpts, so open the full
  message (its ID is a link) before clearing anything you are unsure of. The button "Apply
  the suggestion to the checked" applies the suggestions to the ticked messages in one go.
- **Read the About page**: it shows the contact addresses for corrections, takedown requests
  and access (`nath@logosid.app` and `np2903@tc.columbia.edu`, also in every page's footer) and
  the terms readers accept.
- **Edit the essay** if you wish: `content/home.en.md` and `content/home.id.md` (see below).

## Everyday work

- **New readers** register on `/register` and wait for you on `/admin`: set their access to
  *approved* and Save. A *researcher* also sees the restricted messages.
- **A reader forgot the password**: on `/admin`, press *Reset link* beside the account and send
  the link to the reader yourself (it works once, for 72 hours). The site sends no e-mail.
- **Restrict a message** later: on its page in the reading room, or on `/admin/review`.

## Changing the essay or the program

- The essay is Markdown in `content/home.en.md` (English) and `content/home.id.md`
  (Indonesian). The figures in `{{double braces}}` are filled in from the archive; the comment
  at the top of each file lists them. After editing, **publish again** (the build step takes a
  minute).
- After uploading a new version of the program (`src/…`), run `sh replit-build.sh --analyze` in
  the Shell, then publish again. If the new version says the archive database must be rebuilt,
  run `sh replit-build.sh --rebuild` (15 to 20 minutes).

None of this touches the accounts or your review decisions: they live in the database.

## Keeping the texts private

- The app's files hold every full text. Keep the Replit app **private**, do not make it public,
  do not publish it as a template, and do not invite collaborators you would not give the texts
  to.
- The reading room shows the texts only to approved accounts, and each account can open 1,500
  messages a day. A login lowers the risk of copyright complaints but is not a legal safe
  harbour: answer takedown requests (About page) promptly.
- The GitHub repository holds the program only (see `.gitignore`): never add `data/`,
  `apakabar.db`, `site.db`, `tools/first_pass_*.json` or a `.env` file to it. The database
  password lives only in Replit's secrets (`DATABASE_URL`).
- AI crawlers, AI agents, scrapers and other bots are refused by the server; search engines may
  list the public pages only. Each address may load 300 pages and run 40 searches in 5 minutes.
  The details are in README.md, "Protection of the texts".
- After the archive database is rebuilt (`apakabar ingest`), restart the app so that the
  restricted messages are marked in the new file.

## Problems

| What you see | What to do |
|---|---|
| `No C compiler was found` | Tools → Dependencies → System: add `gcc`, then run `sh replit-build.sh` again. |
| `curl is not available` / Rust does not install | The script installs a current Rust into `.toolchain/` by itself (Replit's own Rust, 1.53, is too old; 1.88 or newer is needed). If that fails, install it in the Shell with `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` and run the build again. |
| The deployment log says `./target/release/apakabar: not found` | The program was built without `build.rs`, so it asks for a loader the published app lacks. Make sure `build.rs` is in the app, run `sh replit-build.sh` in the Shell (it rebuilds the program) and publish again. |
| `The texts are missing` | The `apakabar-data.tar.gz.part-…` files are not in `data/`: upload them. |
| `One of the uploaded files is incomplete or damaged` | Upload the part that `sha256sum` reports as FAILED again. |
| The deployment stops with `DATABASE_URL is not set` | The published app has no database. Add one (step 4), press Run once in the workspace (step 6), make sure the production database is on in the Publish settings, and publish again. |
| The log shows a certificate error for the database | The database offered an encrypted connection whose certificate the program could not check. The message names the two ways out (the database's certificate authority with `sslrootcert=`, or `sslmode=disable` for a database reachable only inside Replit's network); both change the database address, so ask the developer, with the exact message. |
| A form says *Cross-site request refused* on the `….replit.app` address | Use https://apakabar.nathpribady.com, or set a deployment secret `APAKABAR_BASE_URL` to the `….replit.app` address while the domain is not connected. |
| A form says *This form has expired* | The page was open for a long time or cookies were cleared: reload the page and send it again. |
| The build step of a publish takes 20 minutes | The archive database was missing from the published copy and was built again. Run `sh replit-build.sh` in the Shell before publishing, so the publish finds it ready. |
| You forgot the editor password | Add a deployment secret `APAKABAR_ADMIN_RESET` with your e-mail address and publish again: the Logs show a link to set a new password (it works once, for 24 hours). Then delete the secret and publish again. |

## What the scripts run

- `replit-build.sh`: `cargo build --release --locked` (`build.rs` makes the program ask for the
  system's standard loader, `/lib64/ld-linux-x86-64.so.2`, not the one under `/nix/store` that
  Replit's compiler would name and a published app lacks); copies the system libraries the
  program runs on into `runtime/`, for a host without the standard loader; then, only when
  `apakabar.db` is missing, `apakabar ingest --input data --db apakabar.db` and
  `apakabar analyze --db apakabar.db`.
- `replit-start.sh` (published): `apakabar serve --db apakabar.db --addr 0.0.0.0:8080
  --secure-cookies --trust-proxy --require-database --contact "nath@logosid.app,
  np2903@tc.columbia.edu" --base-url https://apakabar.nathpribady.com`. In the workspace the same without `--require-database`,
  with the workspace address as base URL.

Settings that can be changed without touching the scripts (Secrets / deployment secrets):
`APAKABAR_CONTACT` (the addresses on the About page and in the footer, separated by commas) and `APAKABAR_BASE_URL` (the address used
in setup and reset links).
