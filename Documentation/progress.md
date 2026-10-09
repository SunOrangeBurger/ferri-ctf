# FerrisCTF Progress

Spec: FerrisCTF_Technical_Specification_v5.md

## Done

| Item | Status |
| --- | --- |
| Fedora toolchain, project scaffold, `.gitignore` | Done |
| `Cargo.toml` | Done, `cargo check` passes |
| `.env` / `.env.example` | Done |
| `migrations/0001_init.sql` (spec section 4 schema) | Applied, 13 tables verified |
| `src/config.rs`, `src/db.rs`, minimal `src/main.rs` | Verified: WAL on, foreign keys on, secret redacted |
| `src/errors.rs` | Done, user confirmed |
| `migrations/0002_sessions.sql`, `src/keys.rs`, `src/csrf.rs`, `src/session.rs` | Written, 9 tests; **`cargo test` result not yet confirmed** |

## Deviations from the spec

| # | Change | Why |
| --- | --- | --- |
| 1 | Crate is `tower_governor`, not `tower-governor` | Spec has a typo; hyphenated name does not exist on crates.io |
| 2 | Added crates: `rust-embed`, `mime_guess`, `async-trait`, `anyhow`, minijinja `json` feature; dev-deps `http-body-util`, `tempfile` | Spec promises embedded assets but listed no crate for it |
| 3 | Edition 2021; release profile without `panic = "abort"` | Avoid sqlx macro surprises; one handler panic should not kill the event |
| 4 | New env vars `BIND_ADDR`, `COOKIE_SECURE` | Spec describes the behavior but names no variable |
| 5 | Admin credentials are optional in config and checked at bootstrap | Config cannot know whether it is a first run |
| 6 | **Server-side `sessions` table** replaces the signed cookie plus in-memory revocation set (9.1) | Revocation was lost on restart, and the spec says all state lives in SQLite |
| 7 | `is_admin` removed from the cookie; cookie holds only a signed random token; DB stores its SHA-256 | Admin and ban state are read from the DB anyway; a leaked DB cannot be replayed as logins |
| 8 | Max 10 sessions per user | Bounds table growth and abuse |
| 9 | Three subkeys (flag, session, CSRF) derived from `SERVER_SECRET` with domain separation, instead of using the raw secret everywhere | One key's use cannot be turned against another. **Flags will use `keys.flag`** (6.2) |
| 10 | `NotFound` never exposes its message to clients | Enforces the spec's 404 rule at the type level |
| 11 | Cookie has no `Max-Age` | Avoids an extra dependency; the 24h limit is enforced server-side |

## Tabled or open

- Partial unique index so only one round can be active (user tabled until major pieces exist).
- `BEGIN IMMEDIATE` helper: sqlx 0.7 `pool.begin()` is a deferred `BEGIN`. Needed for pool assignment and team joins. Plan: manual helper.
- Anonymous CSRF (login/register have no session): proposed `ferris_pre` cookie binding plus an `Origin`/`Host` check on every POST. Not yet confirmed.
- Auth middleware should redirect browsers to `/login` instead of returning a bare 401.
- Error pages (404, 500) to render through templates once they exist.

## Wiring checklist

- Login: destroy old session, create a new one.
- Logout: `destroy`.
- Password change: `destroy_others_for_user`.
- Ban, force-remove, admin revoke: `destroy_all_for_user`.
- Background task in `main.rs` calling `purge_expired`.

## Next

`AppState` and the auth middleware (cookie to `SessionUser`, 404 admin and Ferris guards, CSRF check).

## Update: state and middleware

- Done: `state.rs`, `middleware/{auth,csrf_guard,admin_guard,mod}.rs`, `routes/mod.rs` (`/health`), `main.rs` serving with graceful shutdown. 22 tests expected.
- Deviation 12: CSRF layer runs before the admin gate, so POSTs to `/admin/*` and unknown paths are indistinguishable.
- Deviation 13: all `/admin/*` is Ferris-only except an admin-level allowlist (fail closed). Path-based gate plus extractors.
- Deviation 14: anonymous CSRF via a `ferris_pre` cookie, issued only when a token is requested.
- Deviation 15: multipart forms send the CSRF token via header or `?csrf_token=` in the action URL.
- Deviation 16: explicit fallback so real 404s match admin 404s.
- Open: `Referrer-Policy` (spec `no-referrer` vs `same-origin`, see Origin check). Decide before the security headers layer.
- Open: 10s shutdown drain cap, rate limiting, 404/500 templates.
- Next: password hashing service (Argon2 + semaphore), Ferris bootstrap, register/login/logout routes.

## Update: passwords and auth service

- Done: `passwords.rs` (Argon2id + semaphore + dummy-hash), `services/auth.rs` (register, login, admin login, change_password, Ferris bootstrap), `state.rs` gains `passwords`. 35 tests expected.
- Deviation 17: usernames lowercase ASCII `[a-z0-9._-]`, 3-32 chars (case-insensitive uniqueness).
- Deviation 18: password policy 8-128 chars (spec silent); applies to Ferris bootstrap.
- Deviation 19: Argon2id explicit params m=19456, t=2, p=1; semaphore num_cpus*2, 5s queue then 503.
- Deviation 20: display names max 40 chars, no control/invisible/bidi characters.
- Deviation 21: bootstrap uses one atomic INSERT, never promotes an existing account.
- Deviation 22: banned status revealed only after the correct password; admin login for non-admins is indistinguishable from bad credentials.
- Cargo: dev profile optimizes dependencies (argon2 is unusable unoptimized).
- Open: Referrer-Policy decision, rate limiting, username charset confirmation.
- Ops: remove ADMIN_PASSWORD from .env after first boot.
- Next: templates, security headers, register/login/logout/admin-login routes.
