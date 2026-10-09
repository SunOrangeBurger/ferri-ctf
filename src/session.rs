use crate::{errors::AppResult, keys::{self, Keys}};
use axum_extra::extract::cookie::{Cookie, SameSite};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::{rngs::OsRng, RngCore};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

pub const COOKIE_NAME: &str = "ferris_session";
/// Hard cap on live sessions per user; oldest are evicted.
pub const MAX_SESSIONS_PER_USER: i64 = 10;
const TOKEN_LEN: usize = 43; // 32 random bytes, base64url, no padding
const MAX_COOKIE_LEN: usize = 128;

/// The authenticated principal, rebuilt from the DB on every request.
/// Nothing here comes from the cookie except the opaque session token.
#[derive(Debug, Clone)]
pub struct SessionUser {
    pub session_id: String, // SHA-256 of the token; also the CSRF binding
    pub user_id: String,
    pub username: String,
    pub display_name: String,
    pub is_admin: bool,
    pub is_ferris: bool,
    pub team_id: Option<String>,
}

fn new_token() -> String {
    let mut b = [0u8; 32];
    OsRng.fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}

fn token_id(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Cookie value = token "." HMAC(session_key, token).
fn seal(keys: &Keys, token: &str) -> String {
    format!("{token}.{}", keys::mac_hex(&keys.session, &[token.as_bytes()]))
}

/// Verify the signature before touching the DB, so forged cookies cost no query.
fn unseal<'a>(keys: &Keys, cookie: &'a str) -> Option<&'a str> {
    if cookie.len() > MAX_COOKIE_LEN {
        return None;
    }
    let (token, sig) = cookie.split_once('.')?;
    if token.len() != TOKEN_LEN {
        return None;
    }
    let expected = keys::mac_hex(&keys.session, &[token.as_bytes()]);
    keys::ct_eq(&expected, sig).then_some(token)
}

/// Create a session row and return the sealed cookie value.
/// Login must call this fresh (and destroy any old session) to rotate the id.
pub async fn create(
    pool: &SqlitePool,
    keys: &Keys,
    ttl_hours: i64,
    user_id: &str,
) -> AppResult<String> {
    let token = new_token();
    let id = token_id(&token);
    let modifier = format!("+{ttl_hours} hours");

    // First statement is a write, so the transaction takes the write lock immediately.
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO sessions (id_hash, user_id, expires_at)
         VALUES (?, ?, strftime('%Y-%m-%d %H:%M:%f', 'now', ?))",
    )
    .bind(&id)
    .bind(user_id)
    .bind(&modifier)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM sessions
         WHERE user_id = ?
           AND id_hash NOT IN (
               SELECT id_hash FROM sessions WHERE user_id = ?
               ORDER BY created_at DESC, rowid DESC LIMIT ?)",
    )
    .bind(user_id)
    .bind(user_id)
    .bind(MAX_SESSIONS_PER_USER)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(seal(keys, &token))
}

/// Resolve a cookie to a user. Returns None for any invalid, expired, unknown or
/// banned session. Admin, Ferris, ban and team state are always read fresh from the DB.
pub async fn authenticate(
    pool: &SqlitePool,
    keys: &Keys,
    cookie: &str,
) -> AppResult<Option<SessionUser>> {
    let Some(token) = unseal(keys, cookie) else {
        return Ok(None);
    };
    let id = token_id(token);

    let row = sqlx::query(
        "SELECT u.id, u.username, u.display_name, u.is_admin, u.is_ferris, tm.team_id
         FROM sessions s
         JOIN users u ON u.id = s.user_id
         LEFT JOIN team_members tm ON tm.user_id = u.id
         WHERE s.id_hash = ?
           AND s.expires_at > strftime('%Y-%m-%d %H:%M:%f', 'now')
           AND u.is_banned = 0",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await?;

    let Some(r) = row else { return Ok(None) };
    let is_admin = r.try_get::<i64, _>("is_admin")? != 0;
    let ferris_flag = r.try_get::<i64, _>("is_ferris")? != 0;
    Ok(Some(SessionUser {
        session_id: id,
        user_id: r.try_get("id")?,
        username: r.try_get("username")?,
        display_name: r.try_get("display_name")?,
        is_admin,
        // Defense in depth: Ferris must also be an admin.
        is_ferris: ferris_flag && is_admin,
        team_id: r.try_get("team_id")?,
    }))
}

/// Logout: kill one session.
pub async fn destroy(pool: &SqlitePool, session_id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM sessions WHERE id_hash = ?")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Ban, forced removal, admin demotion: kill every session the user has.
pub async fn destroy_all_for_user(pool: &SqlitePool, user_id: &str) -> AppResult<u64> {
    let r = sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

/// Password change: kill every session except the current one.
pub async fn destroy_others_for_user(
    pool: &SqlitePool,
    user_id: &str,
    keep_session_id: &str,
) -> AppResult<u64> {
    let r = sqlx::query("DELETE FROM sessions WHERE user_id = ? AND id_hash != ?")
        .bind(user_id)
        .bind(keep_session_id)
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

/// Housekeeping; run periodically from a background task.
pub async fn purge_expired(pool: &SqlitePool) -> AppResult<u64> {
    let r = sqlx::query("DELETE FROM sessions WHERE expires_at <= strftime('%Y-%m-%d %H:%M:%f', 'now')")
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

pub fn cookie(value: String, secure: bool) -> Cookie<'static> {
    Cookie::build((COOKIE_NAME, value))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Strict)
        .secure(secure)
        .build()
}

/// Pass to `CookieJar::remove`. Path must match the one the cookie was set with.
pub fn removal_cookie() -> Cookie<'static> {
    Cookie::build((COOKIE_NAME, "")).path("/").build()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}?mode=rwc", dir.path().join("t.db").display());
        (crate::db::connect(&url).await.unwrap(), dir)
    }

    async fn add_user(pool: &SqlitePool, id: &str) {
        sqlx::query("INSERT INTO users (id, username, display_name, password_hash) VALUES (?, ?, ?, 'x')")
            .bind(id).bind(format!("user-{id}")).bind(format!("User {id}"))
            .execute(pool).await.unwrap();
    }

    fn keys() -> Keys {
        Keys::derive(&[7u8; 32])
    }

    #[tokio::test]
    async fn create_authenticate_destroy() {
        let (pool, _d) = setup().await;
        add_user(&pool, "u1").await;
        let c = create(&pool, &keys(), 24, "u1").await.unwrap();
        let u = authenticate(&pool, &keys(), &c).await.unwrap().expect("valid session");
        assert_eq!(u.user_id, "u1");
        assert!(!u.is_admin && !u.is_ferris && u.team_id.is_none());
        destroy(&pool, &u.session_id).await.unwrap();
        assert!(authenticate(&pool, &keys(), &c).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn forged_or_wrong_key_cookies_rejected() {
        let (pool, _d) = setup().await;
        add_user(&pool, "u1").await;
        let c = create(&pool, &keys(), 24, "u1").await.unwrap();
        let mut bad = c.clone();
        bad.pop();
        bad.push(if c.ends_with('0') { '1' } else { '0' });
        assert!(authenticate(&pool, &keys(), &bad).await.unwrap().is_none());
        let other = Keys::derive(&[9u8; 32]);
        assert!(authenticate(&pool, &other, &c).await.unwrap().is_none());
        for junk in ["", ".", "abc.def", &"x".repeat(500)] {
            assert!(authenticate(&pool, &keys(), junk).await.unwrap().is_none());
        }
    }

    #[tokio::test]
    async fn expired_and_banned_rejected() {
        let (pool, _d) = setup().await;
        add_user(&pool, "u1").await;
        add_user(&pool, "u2").await;
        let c1 = create(&pool, &keys(), 24, "u1").await.unwrap();
        let c2 = create(&pool, &keys(), 24, "u2").await.unwrap();

        sqlx::query("UPDATE sessions SET expires_at = '2000-01-01 00:00:00.000' WHERE user_id = 'u1'")
            .execute(&pool).await.unwrap();
        assert!(authenticate(&pool, &keys(), &c1).await.unwrap().is_none());
        assert_eq!(purge_expired(&pool).await.unwrap(), 1);

        sqlx::query("UPDATE users SET is_banned = 1 WHERE id = 'u2'").execute(&pool).await.unwrap();
        assert!(authenticate(&pool, &keys(), &c2).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn bulk_destroy_and_keep_current() {
        let (pool, _d) = setup().await;
        add_user(&pool, "u1").await;
        let a = create(&pool, &keys(), 24, "u1").await.unwrap();
        let b = create(&pool, &keys(), 24, "u1").await.unwrap();
        let ua = authenticate(&pool, &keys(), &a).await.unwrap().unwrap();
        assert_eq!(destroy_others_for_user(&pool, "u1", &ua.session_id).await.unwrap(), 1);
        assert!(authenticate(&pool, &keys(), &a).await.unwrap().is_some());
        assert!(authenticate(&pool, &keys(), &b).await.unwrap().is_none());
        assert_eq!(destroy_all_for_user(&pool, "u1").await.unwrap(), 1);
        assert!(authenticate(&pool, &keys(), &a).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn per_user_cap_evicts_oldest() {
        let (pool, _d) = setup().await;
        add_user(&pool, "u1").await;
        let first = create(&pool, &keys(), 24, "u1").await.unwrap();
        let mut last = String::new();
        for _ in 0..MAX_SESSIONS_PER_USER + 1 {
            last = create(&pool, &keys(), 24, "u1").await.unwrap();
        }
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE user_id = 'u1'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(n, MAX_SESSIONS_PER_USER);
        assert!(authenticate(&pool, &keys(), &first).await.unwrap().is_none());
        assert!(authenticate(&pool, &keys(), &last).await.unwrap().is_some());
    }
}
