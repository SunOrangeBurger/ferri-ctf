use anyhow::{anyhow, bail};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    passwords::Passwords,
    session,
    state::AppState,
};

pub const USERNAME_MIN: usize = 3;
pub const USERNAME_MAX: usize = 32;
pub const DISPLAY_NAME_MAX_CHARS: usize = 40;
pub const PASSWORD_MIN_CHARS: usize = 8;
pub const PASSWORD_MAX_CHARS: usize = 128;
/// Anything longer is rejected before any hashing work happens.
const PASSWORD_HARD_CAP_BYTES: usize = 1024;

/// Deliberately has no Debug impl: it carries a session cookie value.
pub enum LoginResult {
    Success { user_id: String, cookie_value: String, is_admin: bool },
    /// Unknown user, wrong password, or (on admin login) not an admin. Indistinguishable.
    InvalidCredentials,
    /// Only returned after the correct password was supplied.
    Banned,
}

// ---------- validation ----------

/// Usernames are college IDs: stored lowercase ASCII `[a-z0-9._-]`, so the UNIQUE
/// index also guarantees case-insensitive uniqueness (no "ABC" vs "abc" lookalikes).
pub fn normalize_username(raw: &str) -> AppResult<String> {
    let u = raw.trim().to_ascii_lowercase();
    if u.len() < USERNAME_MIN || u.len() > USERNAME_MAX {
        return Err(AppError::BadRequest(format!(
            "Username must be {USERNAME_MIN} to {USERNAME_MAX} characters"
        )));
    }
    if !u.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) {
        return Err(AppError::BadRequest(
            "Username may contain only letters, digits, '.', '_' and '-'".into(),
        ));
    }
    Ok(u)
}

/// Control, invisible and bidi-override characters could spoof names on the leaderboard.
fn is_forbidden_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
                | '\u{FEFF}'
        )
}

pub fn clean_display_name(raw: &str) -> AppResult<String> {
    let n = raw.trim();
    let count = n.chars().count();
    if count == 0 || count > DISPLAY_NAME_MAX_CHARS {
        return Err(AppError::BadRequest(format!(
            "Display name must be 1 to {DISPLAY_NAME_MAX_CHARS} characters"
        )));
    }
    if n.chars().any(is_forbidden_char) {
        return Err(AppError::BadRequest("Display name contains invalid characters".into()));
    }
    Ok(n.to_owned())
}

pub fn check_password_policy(p: &str) -> AppResult<()> {
    let n = p.chars().count();
    if n < PASSWORD_MIN_CHARS {
        return Err(AppError::BadRequest(format!(
            "Password must be at least {PASSWORD_MIN_CHARS} characters"
        )));
    }
    if n > PASSWORD_MAX_CHARS {
        return Err(AppError::BadRequest(format!(
            "Password must be at most {PASSWORD_MAX_CHARS} characters"
        )));
    }
    Ok(())
}

// ---------- register / login ----------

pub async fn register(
    state: &AppState,
    username: &str,
    display_name: &str,
    password: &str,
) -> AppResult<String> {
    let username = normalize_username(username)?;
    let display_name = clean_display_name(display_name)?;
    check_password_policy(password)?;
    let hash = state.passwords.hash(password.to_owned()).await?;
    let id = Uuid::new_v4().to_string();

    let res = sqlx::query(
        "INSERT INTO users (id, username, display_name, password_hash) VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&username)
    .bind(&display_name)
    .bind(&hash)
    .execute(&state.db)
    .await;

    match res {
        Ok(_) => Ok(id),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            Err(AppError::Conflict("That username is already registered".into()))
        }
        Err(e) => Err(e.into()),
    }
}

fn log_failure(uname: &str, admin_login: bool) {
    let shown: String = uname.chars().take(64).collect();
    // `?` (Debug) escapes newlines, so attacker-supplied text cannot forge log lines.
    tracing::warn!(username = ?shown, admin_login, "login failed");
}

/// `require_admin` is true for /admin/login. `previous_session_id` is the caller's
/// current session, if any; it is destroyed so the session id rotates on login.
pub async fn login(
    state: &AppState,
    username: &str,
    password: &str,
    require_admin: bool,
    previous_session_id: Option<&str>,
) -> AppResult<LoginResult> {
    let uname = username.trim().to_ascii_lowercase();
    if password.len() > PASSWORD_HARD_CAP_BYTES {
        log_failure(&uname, require_admin);
        return Ok(LoginResult::InvalidCredentials);
    }

    let row = if uname.is_empty() || uname.len() > USERNAME_MAX {
        None
    } else {
        sqlx::query("SELECT id, password_hash, is_admin, is_banned FROM users WHERE username = ?")
            .bind(&uname)
            .fetch_optional(&state.db)
            .await?
    };

    let (stored, meta) = match row {
        Some(r) => (
            Some(r.try_get::<String, _>("password_hash")?),
            Some((
                r.try_get::<String, _>("id")?,
                r.try_get::<i64, _>("is_admin")? != 0,
                r.try_get::<i64, _>("is_banned")? != 0,
            )),
        ),
        None => (None, None),
    };

    // One Argon2 verification on every path, including unknown usernames.
    let ok = state.passwords.verify(password.to_owned(), stored).await?;

    let Some((user_id, is_admin, is_banned)) = meta.filter(|_| ok) else {
        log_failure(&uname, require_admin);
        return Ok(LoginResult::InvalidCredentials);
    };
    if require_admin && !is_admin {
        log_failure(&uname, require_admin);
        return Ok(LoginResult::InvalidCredentials);
    }
    if is_banned {
        tracing::warn!(user_id = %user_id, "login attempt by banned user");
        return Ok(LoginResult::Banned);
    }

    if let Some(prev) = previous_session_id {
        session::destroy(&state.db, prev).await?;
    }
    let cookie_value =
        session::create(&state.db, &state.keys, state.cfg.session_ttl_hours, &user_id).await?;
    tracing::info!(user_id = %user_id, admin_login = require_admin, "login ok");
    Ok(LoginResult::Success { user_id, cookie_value, is_admin })
}

/// Verifies the current password, sets the new one and kills every other session
/// in one transaction.
pub async fn change_password(
    state: &AppState,
    user_id: &str,
    current_session_id: &str,
    current_password: &str,
    new_password: &str,
) -> AppResult<()> {
    check_password_policy(new_password)?;
    if current_password.len() > PASSWORD_HARD_CAP_BYTES {
        return Err(AppError::BadRequest("Current password is incorrect".into()));
    }

    let stored: Option<String> = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?;
    let Some(stored) = stored else {
        return Err(AppError::Unauthorized);
    };
    if !state.passwords.verify(current_password.to_owned(), Some(stored)).await? {
        return Err(AppError::BadRequest("Current password is incorrect".into()));
    }

    let new_hash = state.passwords.hash(new_password.to_owned()).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(&new_hash)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = ? AND id_hash != ?")
        .bind(user_id)
        .bind(current_session_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

// ---------- Ferris bootstrap (spec 9.8) ----------

pub async fn bootstrap_ferris(state: &AppState) -> anyhow::Result<()> {
    bootstrap_with(
        &state.db,
        &state.passwords,
        state.cfg.admin_username.as_deref(),
        state.cfg.admin_password.as_deref(),
    )
    .await
}

async fn bootstrap_with(
    db: &SqlitePool,
    passwords: &Passwords,
    username: Option<&str>,
    password: Option<&str>,
) -> anyhow::Result<()> {
    let admins: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE is_admin = 1")
        .fetch_one(db)
        .await?;
    if admins > 0 {
        let ferris: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE is_ferris = 1")
            .fetch_one(db)
            .await?;
        if ferris == 0 {
            tracing::error!("admins exist but no Ferris account does; Ferris-only routes are unreachable");
        }
        return Ok(());
    }

    let (Some(username), Some(password)) = (username, password) else {
        bail!("no admin account exists: set ADMIN_USERNAME and ADMIN_PASSWORD in the environment for first boot");
    };
    let username = normalize_username(username).map_err(|e| anyhow!("ADMIN_USERNAME rejected: {e}"))?;
    check_password_policy(password).map_err(|e| anyhow!("ADMIN_PASSWORD rejected: {e}"))?;
    let hash = passwords.hash(password.to_owned()).await?;

    // One atomic statement: inserts only if there is still no admin.
    let res = sqlx::query(
        "INSERT INTO users (id, username, display_name, password_hash, is_admin, is_ferris)
         SELECT ?, ?, 'Ferris', ?, 1, 1
         WHERE NOT EXISTS (SELECT 1 FROM users WHERE is_admin = 1)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&username)
    .bind(&hash)
    .execute(db)
    .await;

    match res {
        Ok(r) if r.rows_affected() == 1 => {
            tracing::warn!(
                username = %username,
                "Ferris account created from the environment. Change this password after logging in, \
                 then remove ADMIN_PASSWORD from .env"
            );
        }
        Ok(_) => {}
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            bail!(
                "ADMIN_USERNAME '{username}' belongs to an existing non-admin account; \
                 refusing to promote it automatically"
            );
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::test_support;

    const PW: &str = "correct horse battery";

    #[test]
    fn usernames() {
        assert_eq!(normalize_username("  21BCE1234 ").unwrap(), "21bce1234");
        assert_eq!(normalize_username("a.b_c-d").unwrap(), "a.b_c-d");
        for bad in ["", "ab", &"a".repeat(33), "has space", "semi;colon", "új", "a/b", "../x", "a\nb"] {
            assert!(normalize_username(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn display_names() {
        assert_eq!(clean_display_name("  Ada Lovelace ").unwrap(), "Ada Lovelace");
        assert!(clean_display_name("   ").is_err());
        assert!(clean_display_name(&"x".repeat(41)).is_err());
        assert!(clean_display_name(&"x".repeat(40)).is_ok());
        for bad in ["a\nb", "a\tb", "evil\u{202E}name", "zero\u{200B}width", "bom\u{FEFF}x", "nul\0x"] {
            assert!(clean_display_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn password_policy() {
        assert!(check_password_policy("1234567").is_err());
        assert!(check_password_policy("12345678").is_ok());
        assert!(check_password_policy(&"p".repeat(128)).is_ok());
        assert!(check_password_policy(&"p".repeat(129)).is_err());
    }

    #[tokio::test]
    async fn register_then_login_case_insensitive() {
        let (s, _d) = test_support::state().await;
        let id = register(&s, "21BCE1234", "Ada", PW).await.unwrap();
        let stored: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
            .bind(&id)
            .fetch_one(&s.db)
            .await
            .unwrap();
        assert!(stored.starts_with("$argon2id$"));
        assert!(!stored.contains(PW));

        let LoginResult::Success { user_id, cookie_value, is_admin } =
            login(&s, " 21bce1234 ", PW, false, None).await.unwrap()
        else {
            panic!("login failed");
        };
        assert_eq!(user_id, id);
        assert!(!is_admin);
        assert!(session::authenticate(&s.db, &s.keys, &cookie_value).await.unwrap().is_some());

        let dup = register(&s, "21bce1234", "Other", PW).await;
        assert!(matches!(dup, Err(AppError::Conflict(_))), "case-variant username must conflict");
    }

    #[tokio::test]
    async fn login_failure_modes() {
        let (s, _d) = test_support::state().await;
        let id = register(&s, "ada", "Ada", PW).await.unwrap();
        let bad = "wrong-password";
        assert!(matches!(login(&s, "ada", bad, false, None).await.unwrap(), LoginResult::InvalidCredentials));
        assert!(matches!(login(&s, "nobody", PW, false, None).await.unwrap(), LoginResult::InvalidCredentials));
        assert!(matches!(login(&s, "", PW, false, None).await.unwrap(), LoginResult::InvalidCredentials));
        assert!(matches!(login(&s, &"a".repeat(500), PW, false, None).await.unwrap(), LoginResult::InvalidCredentials));
        assert!(matches!(login(&s, "ada", &"p".repeat(5000), false, None).await.unwrap(), LoginResult::InvalidCredentials));

        sqlx::query("UPDATE users SET is_banned = 1 WHERE id = ?").bind(&id).execute(&s.db).await.unwrap();
        assert!(matches!(login(&s, "ada", PW, false, None).await.unwrap(), LoginResult::Banned));
        assert!(
            matches!(login(&s, "ada", bad, false, None).await.unwrap(), LoginResult::InvalidCredentials),
            "ban status must not leak without the correct password"
        );
    }

    #[tokio::test]
    async fn login_rotates_the_session() {
        let (s, _d) = test_support::state().await;
        register(&s, "ada", "Ada", PW).await.unwrap();
        let LoginResult::Success { cookie_value: a, .. } = login(&s, "ada", PW, false, None).await.unwrap() else {
            panic!("login failed");
        };
        let sid = session::authenticate(&s.db, &s.keys, &a).await.unwrap().unwrap().session_id;
        let LoginResult::Success { cookie_value: b, .. } = login(&s, "ada", PW, false, Some(&sid)).await.unwrap() else {
            panic!("login failed");
        };
        assert!(session::authenticate(&s.db, &s.keys, &a).await.unwrap().is_none(), "old session destroyed");
        assert!(session::authenticate(&s.db, &s.keys, &b).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn admin_login_requires_admin_flag() {
        let (s, _d) = test_support::state().await;
        register(&s, "player", "P", PW).await.unwrap();
        bootstrap_with(&s.db, &s.passwords, Some("ferris"), Some(PW)).await.unwrap();

        assert!(matches!(login(&s, "player", PW, true, None).await.unwrap(), LoginResult::InvalidCredentials));
        assert!(matches!(login(&s, "player", PW, false, None).await.unwrap(), LoginResult::Success { .. }));
        let LoginResult::Success { is_admin, .. } = login(&s, "ferris", PW, true, None).await.unwrap() else {
            panic!("ferris admin login failed");
        };
        assert!(is_admin);
    }

    #[tokio::test]
    async fn change_password_rotates_other_sessions() {
        let (s, _d) = test_support::state().await;
        let id = register(&s, "ada", "Ada", PW).await.unwrap();
        let LoginResult::Success { cookie_value: a, .. } = login(&s, "ada", PW, false, None).await.unwrap() else {
            panic!("login failed");
        };
        let LoginResult::Success { cookie_value: b, .. } = login(&s, "ada", PW, false, None).await.unwrap() else {
            panic!("login failed");
        };
        let sid_a = session::authenticate(&s.db, &s.keys, &a).await.unwrap().unwrap().session_id;
        let new_pw = "new password 123";

        assert!(matches!(change_password(&s, &id, &sid_a, "wrong-password", new_pw).await, Err(AppError::BadRequest(_))));
        assert!(matches!(change_password(&s, &id, &sid_a, PW, "short").await, Err(AppError::BadRequest(_))));
        change_password(&s, &id, &sid_a, PW, new_pw).await.unwrap();

        assert!(session::authenticate(&s.db, &s.keys, &a).await.unwrap().is_some(), "current session kept");
        assert!(session::authenticate(&s.db, &s.keys, &b).await.unwrap().is_none(), "other sessions killed");
        assert!(matches!(login(&s, "ada", PW, false, None).await.unwrap(), LoginResult::InvalidCredentials));
        assert!(matches!(login(&s, "ada", new_pw, false, None).await.unwrap(), LoginResult::Success { .. }));
    }

    #[tokio::test]
    async fn bootstrap_creates_ferris_once() {
        let (s, _d) = test_support::state().await;
        bootstrap_with(&s.db, &s.passwords, Some("Ferris-ID"), Some(PW)).await.unwrap();
        let row = sqlx::query("SELECT username, is_admin, is_ferris, password_hash FROM users")
            .fetch_one(&s.db)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("username"), "ferris-id");
        assert_eq!(row.get::<i64, _>("is_admin"), 1);
        assert_eq!(row.get::<i64, _>("is_ferris"), 1);
        assert!(row.get::<String, _>("password_hash").starts_with("$argon2id$"));

        // Later boots are no-ops, with or without the env vars.
        bootstrap_with(&s.db, &s.passwords, Some("someone-else"), Some(PW)).await.unwrap();
        bootstrap_with(&s.db, &s.passwords, None, None).await.unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users").fetch_one(&s.db).await.unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn bootstrap_failure_modes() {
        let (s, _d) = test_support::state().await;
        assert!(bootstrap_with(&s.db, &s.passwords, None, None).await.is_err());
        assert!(bootstrap_with(&s.db, &s.passwords, Some("ferris"), None).await.is_err());
        assert!(bootstrap_with(&s.db, &s.passwords, Some("ferris"), Some("short")).await.is_err());
        assert!(bootstrap_with(&s.db, &s.passwords, Some("a b"), Some(PW)).await.is_err());

        register(&s, "taken", "T", PW).await.unwrap();
        assert!(bootstrap_with(&s.db, &s.passwords, Some("taken"), Some(PW)).await.is_err());
        let admins: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE is_admin = 1")
            .fetch_one(&s.db)
            .await
            .unwrap();
        assert_eq!(admins, 0, "an existing account must never be promoted silently");
    }
}
