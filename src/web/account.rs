//! A reader's own account: changing the password, and setting a new one through a reset link
//! an editor made. Also the one-time setup page that creates the first editor account.

use super::auth::{self, MaybeUser, Reader, MIN_PASSWORD};
use super::i18n::{t, tf};
use super::layout::{csrf_field, page};
use super::security::{self, ClientIp};
use super::{site, AppError, AppState};
use axum::{
    extract::{Form, Path, Query, State},
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::CookieJar;
use maud::{html, Markup};
use serde::Deserialize;

fn flash(msg: Option<(&str, bool)>) -> Markup {
    html! { @if let Some((m, ok)) = msg { div class=(if ok { "flash ok" } else { "flash" }) { (m) } } }
}

fn too_short() -> String {
    tf(|| format!("Choose a password of at least {MIN_PASSWORD} characters, and type it twice the same way."), || format!("Pilih kata sandi paling sedikit {MIN_PASSWORD} karakter, dan ketik dua kali dengan sama persis."))
}

// ---------------------------------------------------------------- change password

fn account_page(u: &auth::User, msg: Option<(&str, bool)>) -> Markup {
    let body = html! {
        div.essay style="max-width:480px" {
            h1 { (t("Your account", "Akun Anda")) }
            p { (u.email) " · " (match u.role.as_str() { "admin" => t("editor", "editor"), "researcher" => t("researcher", "peneliti"), _ => t("reader", "pembaca") }) }
            (flash(msg))
            h2 { (t("Change your password", "Ganti kata sandi")) }
            form.stack method="post" action="/account" {
                (csrf_field())
                label for="current" { (t("Current password", "Kata sandi sekarang")) } input type="password" id="current" name="current" required autocomplete="current-password";
                label for="password" { (tf(|| format!("New password (at least {MIN_PASSWORD} characters)"), || format!("Kata sandi baru (paling sedikit {MIN_PASSWORD} karakter)"))) } input type="password" id="password" name="password" required minlength=(MIN_PASSWORD) autocomplete="new-password";
                label for="again" { (t("New password again", "Ulangi kata sandi baru")) } input type="password" id="again" name="again" required minlength=(MIN_PASSWORD) autocomplete="new-password";
                p { button.btn type="submit" { (t("Change password", "Ganti kata sandi")) } }
            }
            p.small { (t("Changing the password signs you out everywhere else.", "Mengganti kata sandi akan mengeluarkan Anda dari semua perangkat lain.")) }
        }
    };
    page(t("Your account", "Akun Anda"), Some(u), "account", body)
}

pub async fn account_form(Reader(u): Reader) -> Markup {
    account_page(&u, None)
}

#[derive(Deserialize)]
pub struct PwForm {
    current: String,
    password: String,
    again: String,
}

pub async fn change_password(State(st): State<AppState>, Reader(u): Reader, jar: CookieJar, Form(f): Form<PwForm>) -> Result<Response, AppError> {
    let key = format!("pw-acct:{}", u.id);
    if st.throttle.recent(&key, security::LOGIN_WINDOW) >= security::LOGIN_PER_ACCOUNT {
        return Ok(account_page(&u, Some((t("Too many attempts. Please wait 15 minutes.", "Terlalu banyak percobaan. Tunggu 15 menit."), false))).into_response());
    }
    if f.password.chars().count() < MIN_PASSWORD || f.password != f.again {
        return Ok(account_page(&u, Some((&too_short(), false))).into_response());
    }
    let uid = u.id;
    let acct = site(&st, move |c| c.account_by_id(uid)).await?;
    // counted before the check, so attempts sent all at once are counted too
    st.throttle.note(&key);
    if !auth::check_password(f.current.clone(), acct.map(|a| a.pw_hash)).await {
        return Ok(account_page(&u, Some((t("The current password is not right.", "Kata sandi sekarang tidak tepat."), false))).into_response());
    }
    st.throttle.forget_last(&key);
    let hash = auth::new_hash(f.password.clone()).await?;
    let token = site(&st, move |c| {
        c.set_password(uid, &hash)?;
        c.delete_sessions_of(uid)?;
        c.audit(uid, "password changed", "")?;
        c.create_session(uid)
    })
    .await?;
    let jar = jar.add(auth::session_cookie(token, st.secure_cookies));
    Ok((jar, account_page(&u, Some((t("Your password has been changed.", "Kata sandi Anda sudah diganti."), true)))).into_response())
}

// ---------------------------------------------------------------- reset links

fn reset_page(user: Option<&auth::User>, token: &str, email: Option<&str>, msg: Option<(&str, bool)>, done: bool) -> Markup {
    let body = html! {
        div.essay style="max-width:480px" {
            h1 { (t("Set a new password", "Buat kata sandi baru")) }
            (flash(msg))
            @if done {
                p { a.btn href="/login" { (t("Log in", "Masuk")) } }
            } @else if let Some(e) = email {
                p { (t("For", "Untuk")) " " b { (e) } "." }
                form.stack method="post" action=(format!("/reset/{token}")) {
                    (csrf_field())
                    label for="password" { (tf(|| format!("New password (at least {MIN_PASSWORD} characters)"), || format!("Kata sandi baru (paling sedikit {MIN_PASSWORD} karakter)"))) } input type="password" id="password" name="password" required minlength=(MIN_PASSWORD) autocomplete="new-password";
                    label for="again" { (t("New password again", "Ulangi kata sandi baru")) } input type="password" id="again" name="again" required minlength=(MIN_PASSWORD) autocomplete="new-password";
                    p { button.btn type="submit" { (t("Save the new password", "Simpan kata sandi baru")) } }
                }
            } @else {
                p { (t("This link has expired or has already been used. Ask an editor for a new one.", "Tautan ini sudah kedaluwarsa atau sudah dipakai. Mintalah tautan baru kepada editor.")) }
            }
        }
    };
    page(t("New password", "Kata sandi baru"), user, "account", body)
}

pub async fn reset_form(State(st): State<AppState>, MaybeUser(user): MaybeUser, Path(token): Path<String>) -> Result<Markup, AppError> {
    let tk = token.clone();
    let acct = site(&st, move |c| c.reset_account(&tk)).await?;
    Ok(reset_page(user.as_ref(), &token, acct.as_ref().map(|a| a.email.as_str()), None, false))
}

#[derive(Deserialize)]
pub struct ResetForm {
    password: String,
    again: String,
}

pub async fn reset(State(st): State<AppState>, ClientIp(ip): ClientIp, Path(token): Path<String>, Form(f): Form<ResetForm>) -> Result<Markup, AppError> {
    let key = format!("reset-ip:{ip}");
    if st.throttle.recent(&key, security::REGISTER_WINDOW) >= security::LOGIN_PER_IP {
        return Ok(reset_page(None, &token, None, Some((t("Too many attempts. Please wait an hour.", "Terlalu banyak percobaan. Tunggu satu jam."), false)), true));
    }
    st.throttle.note(&key);
    let tk = token.clone();
    let acct = site(&st, move |c| c.reset_account(&tk)).await?;
    let Some(acct) = acct else {
        return Ok(reset_page(None, &token, None, None, false));
    };
    if f.password.chars().count() < MIN_PASSWORD || f.password != f.again {
        return Ok(reset_page(None, &token, Some(&acct.email), Some((&too_short(), false)), false));
    }
    let hash = auth::new_hash(f.password.clone()).await?;
    let tk = token.clone();
    let done = site(&st, move |c| {
        let r = c.use_reset(&tk, &hash)?;
        if let Some(uid) = r {
            c.audit(uid, "password reset", "")?;
        }
        Ok(r)
    })
    .await?;
    match done {
        Some(_) => Ok(reset_page(None, &token, None, Some((t("Your new password is saved. You can log in now.", "Kata sandi baru Anda tersimpan. Silakan masuk."), true)), true)),
        None => Ok(reset_page(None, &token, None, None, false)),
    }
}

// ---------------------------------------------------------------- the first editor

#[derive(Deserialize, Default)]
pub struct SetupQ {
    #[serde(default)]
    token: String,
}

fn setup_valid(st: &AppState, token: &str) -> bool {
    st.setup.lock().unwrap().as_deref().map(|t| security::same(t, token)).unwrap_or(false)
}

fn setup_page(token: &str, msg: Option<(&str, bool)>) -> Markup {
    let body = html! {
        div.essay style="max-width:520px" {
            h1 { (t("Create the editor account", "Buat akun editor")) }
            (flash(msg))
            p { (t("This page works once, while the site has no editor. The account you create can approve readers, restrict messages and review the precision checks.", "Halaman ini hanya berlaku sekali, selama situs belum punya editor. Akun yang Anda buat dapat menyetujui pembaca, membatasi pesan dan meninjau pemeriksaan presisi.")) }
            form.stack method="post" action="/setup" {
                (csrf_field())
                input type="hidden" name="token" value=(token);
                label for="name" { (t("Name", "Nama")) } input type="text" id="name" name="name" required maxlength="120";
                label for="email" { "Email" } input type="email" id="email" name="email" required autocomplete="username";
                label for="password" { (tf(|| format!("Password (at least {MIN_PASSWORD} characters)"), || format!("Kata sandi (paling sedikit {MIN_PASSWORD} karakter)"))) } input type="password" id="password" name="password" required minlength=(MIN_PASSWORD) autocomplete="new-password";
                label for="again" { (t("Password again", "Ulangi kata sandi")) } input type="password" id="again" name="again" required minlength=(MIN_PASSWORD) autocomplete="new-password";
                p { button.btn type="submit" { (t("Create the account", "Buat akun")) } }
            }
        }
    };
    page(t("Set up", "Persiapan"), None, "account", body)
}

pub async fn setup_form(State(st): State<AppState>, Query(q): Query<SetupQ>) -> Response {
    if !setup_valid(&st, &q.token) {
        return Redirect::to("/login").into_response();
    }
    setup_page(&q.token, None).into_response()
}

#[derive(Deserialize)]
pub struct SetupForm {
    token: String,
    name: String,
    email: String,
    password: String,
    again: String,
}

pub async fn setup(State(st): State<AppState>, jar: CookieJar, Form(f): Form<SetupForm>) -> Result<Response, AppError> {
    if !setup_valid(&st, &f.token) {
        return Ok(Redirect::to("/login").into_response());
    }
    let email = f.email.trim().to_lowercase();
    if f.password.chars().count() < MIN_PASSWORD || f.password != f.again || !email.contains('@') || f.name.trim().is_empty() {
        return Ok(setup_page(&f.token, Some((&too_short(), false))).into_response());
    }
    // the link works once: it is taken before the account is made, so two forms sent at the
    // same moment cannot both create one
    let claimed = {
        let mut s = st.setup.lock().unwrap();
        if s.as_deref().map(|t| security::same(t, &f.token)).unwrap_or(false) { s.take() } else { None }
    };
    let Some(link) = claimed else {
        return Ok(Redirect::to("/login").into_response());
    };
    let made = async {
        let hash = auth::new_hash(f.password.clone()).await?;
        let name = f.name.trim().to_string();
        site(&st, move |c| {
            if c.count_admins()? > 0 {
                return Ok(None);
            }
            c.upsert_account(&email, &name, &hash, "admin")?;
            let id = c.account_by_email(&email)?.map(|a| a.id).unwrap_or(0);
            c.audit(id, "editor account created", "setup link")?;
            Ok(Some(c.create_session(id)?))
        })
        .await
    }
    .await;
    let token = match made {
        Ok(t) => t,
        Err(e) => {
            // nothing was made: the link stays valid
            *st.setup.lock().unwrap() = Some(link);
            return Err(e);
        }
    };
    match token {
        Some(tok) => Ok((jar.add(auth::session_cookie(tok, st.secure_cookies)), Redirect::to("/admin")).into_response()),
        None => Ok(Redirect::to("/login").into_response()),
    }
}
