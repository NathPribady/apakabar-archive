//! Accounts and sessions for the reading room. Passwords are hashed with Argon2id; sessions
//! are random tokens in an HttpOnly cookie, and the site store keeps only their SHA-256.

use super::AppState;
use crate::{ingest::anyhow_lite::Result, site::SiteDb};
use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use std::path::Path;

pub const COOKIE: &str = "ak_session";
pub const MIN_PASSWORD: usize = 10;

#[derive(Clone, Debug)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub name: String,
    pub role: String,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
    pub fn can_see_sensitive(&self) -> bool {
        self.role == "admin" || self.role == "researcher"
    }
}

pub fn hash_password(pw: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default().hash_password(pw.as_bytes(), &salt).expect("hash").to_string()
}

pub fn verify_password(pw: &str, hash: &str) -> bool {
    PasswordHash::new(hash).map(|h| Argon2::default().verify_password(pw.as_bytes(), &h).is_ok()).unwrap_or(false)
}

/// Spend the same time on a wrong email as on a wrong password.
pub fn verify_dummy(pw: &str) {
    let _ = verify_password(pw, "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$Xj3oL9pr8Oe1qY3Qx2Q0kEo3m5pY8n1d3c0k8Z0n2aA");
}

/// Argon2 takes 19 MiB of memory per password. The server works on two at a time, so a
/// flood of login attempts waits its turn instead of using up the machine's memory.
static HASHING: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

/// Check a password off the async threads, two at a time. Without an account (`None`) the
/// same work is done on a dummy hash, so a wrong email takes as long as a wrong password.
pub async fn check_password(pw: String, hash: Option<String>) -> bool {
    let _turn = HASHING.acquire().await.ok();
    tokio::task::spawn_blocking(move || match hash {
        Some(h) => verify_password(&pw, &h),
        None => {
            verify_dummy(&pw);
            false
        }
    })
    .await
    .unwrap_or(false)
}

/// Hash a new password off the async threads, two at a time.
pub async fn new_hash(pw: String) -> std::result::Result<String, super::AppError> {
    let _turn = HASHING.acquire().await.ok();
    tokio::task::spawn_blocking(move || hash_password(&pw)).await.map_err(|e| super::AppError(e.to_string()))
}

pub fn session_cookie(token: String, secure: bool) -> Cookie<'static> {
    let mut ck = Cookie::new(COOKIE, token);
    ck.set_http_only(true);
    ck.set_same_site(SameSite::Lax);
    ck.set_path("/");
    ck.set_secure(secure);
    ck.set_max_age(time::Duration::days(crate::site::SESSION_DAYS));
    ck
}

/// The signed-in user, if any (for public pages that change their header).
pub struct MaybeUser(pub Option<User>);

#[axum::async_trait]
impl FromRequestParts<AppState> for MaybeUser {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> std::result::Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let tok = jar.get(COOKIE).map(|c| c.value().to_string());
        let user = match tok {
            Some(t) => {
                let db = state.site.clone();
                tokio::task::spawn_blocking(move || {
                    let mut c = db.get().ok()?;
                    c.account_for_session(&t).ok().flatten().map(|a| User { id: a.id, email: a.email, name: a.name, role: a.role })
                })
                .await
                .ok()
                .flatten()
            }
            None => None,
        };
        Ok(MaybeUser(user))
    }
}

/// A signed-in reader; otherwise a redirect to the login page.
pub struct Reader(pub User);

pub struct LoginRedirect(String);
impl IntoResponse for LoginRedirect {
    fn into_response(self) -> Response {
        Redirect::to(&format!("/login?next={}", urlencoding::encode(&self.0))).into_response()
    }
}

#[axum::async_trait]
impl FromRequestParts<AppState> for Reader {
    type Rejection = LoginRedirect;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> std::result::Result<Self, Self::Rejection> {
        let MaybeUser(u) = MaybeUser::from_request_parts(parts, state).await.unwrap();
        let next = parts.uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_else(|| "/read".into());
        u.map(Reader).ok_or(LoginRedirect(next))
    }
}

pub struct Admin(pub User);
#[axum::async_trait]
impl FromRequestParts<AppState> for Admin {
    type Rejection = Response;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> std::result::Result<Self, Self::Rejection> {
        let r = Reader::from_request_parts(parts, state).await.map_err(|e| e.into_response())?;
        if r.0.is_admin() {
            Ok(Admin(r.0))
        } else {
            Err((StatusCode::FORBIDDEN, super::i18n::t("Editors only", "Khusus editor").to_string()).into_response())
        }
    }
}

/// Command-line account creation or update. Uses PostgreSQL when DATABASE_URL is set,
/// otherwise site.db next to the archive database.
pub fn cli_user(db_path: &Path, site: Option<&Path>, email: &str, name: &str, role: &str, password: Option<String>) -> Result<()> {
    if !["reader", "researcher", "admin"].contains(&role) {
        return Err("role must be reader, researcher or admin".into());
    }
    let pw = match password {
        Some(p) => p,
        None => {
            eprint!("Password for {email}: ");
            let mut s = String::new();
            std::io::stdin().read_line(&mut s)?;
            s.trim().to_string()
        }
    };
    if pw.chars().count() < MIN_PASSWORD {
        return Err(format!("password must have at least {MIN_PASSWORD} characters").into());
    }
    let db = SiteDb::for_archive(db_path, site, None)?;
    db.get()?.upsert_account(&email.trim().to_lowercase(), name, &hash_password(&pw), role)?;
    eprintln!("account {email} saved with role {role} in {}", db.describe());
    Ok(())
}
