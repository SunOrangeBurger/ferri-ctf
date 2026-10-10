# FerrisCTF 🦀 Testing & Verification Guide

This document details the complete testing strategy, test suites, automated execution results, and manual verification steps for the FerrisCTF platform.

---

## 📋 Executive Summary

The FerrisCTF platform has undergone end-to-end verification covering core functionality, security controls, operator workflows, and player lifecycles.

| Test Category | Suite / Runner | Tests Executed | Passed | Failed | Status |
|---|---|---|---|---|---|
| **Unit & Integration Tests** | `cargo test` | 55 | 55 | 0 | ✅ **100% Passed** |
| **End-to-End System Tests** | `test_full_suite.py` | 44 | 44 | 0 | ✅ **100% Passed** |
| **Code Linting & Types** | `cargo check` / `clippy` | — | — | — | ✅ **Clean** |

---

## ⚙️ Environment & Setup

FerrisCTF runs as a self-contained Rust binary with an embedded SQLite database, MiniJinja templates, and static assets.

### 1. Prerequisites
- Rust 1.75+ (`rustc`, `cargo`)
- SQLite 3 runtime library

### 2. Configuration (`.env`)
```ini
SERVER_SECRET=3a557e4b885b3468d265efcf2f6d7fa6746d61564eb53342b1a6da8fb79a6dd0
ADMIN_USERNAME=PES2UG24AM126
ADMIN_PASSWORD=9c4abc0b6b657d47b3d565adc07f81c7
DATABASE_URL=sqlite://ferrisctf.db?mode=rwc
BIND_ADDR=0.0.0.0:8080
COOKIE_SECURE=false
RUST_LOG=info,ferrisctf=debug
```

### 3. Launching the Service
To compile and start the server in development mode:
```bash
cargo run
```
The server binds to `0.0.0.0:8080`. SQLite migrations run automatically on startup.

---

## 🧪 Automated Test Suites

### 1. Cargo Unit & Integration Suite (`cargo test`)
Runs 55 unit and integration tests verifying cryptographic subkeys, CSRF token binding, rate limiting sliding windows, Argon2id passwords, session lifecycle, flag verifiers, pool allocation, and route security.

```bash
cargo test
```

#### Output Summary:
```text
running 55 tests
test keys::tests::ct_eq_basics ... ok
test csrf::tests::roundtrip_and_binding ... ok
test csrf::tests::rejects_tampering_and_garbage ... ok
test middleware::admin_guard::tests::evasion_attempts_are_still_gated ... ok
test middleware::admin_guard::tests::levels ... ok
test keys::tests::subkeys_are_distinct_and_deterministic ... ok
test middleware::csrf_guard::tests::origin_rules ... ok
test middleware::csrf_guard::tests::field_parsing ... ok
test ratelimit::tests::window_slides_and_keys_are_isolated ... ok
test ratelimit::tests::allows_up_to_limit_then_blocks ... ok
test ratelimit::tests::ip_keys ... ok
test passwords::tests::unknown_user_and_garbage_hashes_never_verify ... ok
test middleware::tests::admin_login_is_public_but_csrf_protected ... ok
test middleware::tests::pre_cookie_is_only_issued_when_a_token_is_requested ... ok
test middleware::tests::ferris_flag_without_admin_is_not_ferris ... ok
test middleware::tests::auth_extractors ... ok
test passwords::tests::saturated_pool_fails_with_503 ... ok
test middleware::tests::admin_and_ferris_levels ... ok
test middleware::tests::admin_routes_are_indistinguishable_from_missing_routes ... ok
test middleware::tests::csrf_token_transports_origin_and_size_limits ... ok
test services::auth::tests::display_names ... ok
test middleware::tests::csrf_roundtrip_and_rejections ... ok
test middleware::tests::csrf_tokens_are_bound_to_the_session ... ok
test services::auth::tests::password_policy ... ok
test routes::auth::tests::security_headers_and_static_files ... ok
test services::auth::tests::usernames ... ok
test services::files::tests::test_sanitize_header_filename ... ok
test services::files::tests::test_validate_and_store_zip ... ok
test services::pool::tests::test_difficulty_quotas ... ok
test ratelimit::tests::key_cap_fails_closed_then_recovers ... ok
test passwords::tests::hash_and_verify ... ok
test routes::auth::tests::register_validation_and_conflicts ... ok
test services::verifier::tests::test_flag_verification_and_trimming ... ok
test services::verifier::tests::test_hash_submission ... ok
test session::tests::bulk_destroy_and_keep_current ... ok
test services::auth::tests::bootstrap_failure_modes ... ok
test session::tests::create_authenticate_destroy ... ok
test services::auth::tests::bootstrap_creates_ferris_once ... ok
test session::tests::expired_and_banned_rejected ... ok
test templates::tests::all_required_templates_compile ... ok
test routes::auth::tests::error_pages_are_themed_and_identical ... ok
test session::tests::forged_or_wrong_key_cookies_rejected ... ok
test session::tests::per_user_cap_evicts_oldest ... ok
test routes::auth::tests::admin_login_flow_and_rate_limit ... ok
test services::rounds::tests::test_rounds_lifecycle ... ok
test routes::auth::tests::output_is_html_escaped ... ok
test routes::auth::tests::register_login_logout_flow ... ok
test services::scoring::tests::test_scoring_and_tie_breaks ... ok
test services::auth::tests::register_then_login_case_insensitive ... ok
test services::auth::tests::login_rotates_the_session ... ok
test services::team::tests::test_team_lifecycle ... ok
test services::auth::tests::admin_login_requires_admin_flag ... ok
test services::auth::tests::change_password_rotates_other_sessions ... ok
test services::auth::tests::login_failure_modes ... ok
test routes::auth::tests::login_failures_ban_and_rate_limit ... ok

test result: ok. 55 passed; 0 failed; 0 ignored; 0 measured; finished in 0.43s
```

---

### 2. Live HTTP End-to-End Suite (`test_full_suite.py`)
Simulates concurrent anonymous visitors, admins, team captains, and teammates interacting with the live HTTP server.

```bash
python3 scratch/test_full_suite.py
```

#### Test Execution Breakdown:

```text
======================================================================
FERRISCTF COMPLETE END-TO-END VERIFICATION
======================================================================
[PASS] Health Check /health - Status 200
[PASS] Public Scoreboard Page /scoreboard - Status 200
[PASS] Scoreboard API /api/scoreboard - Status 200
[PASS] Static Asset /static/style.css - Status 200
[PASS] Admin Route Isolation Dashboard (Anon -> 404) - Got status 404
[PASS] Admin Route Isolation Audit (Anon -> 404) - Got status 404
[PASS] Admin Login Page & CSRF Token - CSRF present: True
[PASS] Admin Bootstrap Authentication - Redirect/Dashboard status 200
[PASS] Admin Dashboard Access /admin/dashboard - Status 200
[PASS] Admin Section /admin/teams - Status 200
[PASS] Admin Section /admin/rounds - Status 200
[PASS] Admin Section /admin/challenges - Status 200
[PASS] Admin Section /admin/audit - Status 200
[PASS] Admin Section /admin/admins - Status 200
[PASS] Admin Export Submissions CSV - Status 200
[PASS] Admin Export Solves CSV - Status 200
[PASS] Admin Create Challenge (Multipart) - Status 200
[PASS] Admin Challenges List Contains New Challenge - Found challenge in list
[PASS] Admin Add Hint to Challenge - Status 200
[PASS] Player 1 Registration - Registered player
[PASS] Player 1 Onboarding Page - Status 200
[PASS] Player 1 Create Team - Team Titans
[PASS] Player 1 Team Dashboard - Status 200
[PASS] Team Join Code Generated - 8-character Crockford Base32
[PASS] Admin Unlock Team for Roster Joining - Status 200
[PASS] Player 2 Registration - Registered teammate
[PASS] Player 2 Join Team via Join Code - Status 200
[PASS] Player 2 In Team Dashboard - Both members in team roster
[PASS] Admin Lock Team - Status 200
[PASS] Admin End Current Round - Ended round 1
[PASS] Admin Start Next Round & Assign Challenge Pools - Status 200
[PASS] Player View Challenges List (Team Locked & Round Active) - Status 200
[PASS] Player Challenge Pool Contains Challenge - Quota matched
[PASS] Player View Challenge Detail - Status 200
[PASS] Player Request Hint (-10 pts) - Status 200
[PASS] Incorrect Flag Submission Handled - Status 200
[PASS] Correct Flag Submission Accepted - Status 200
[PASS] Challenge Page Confirms Solve - Solve badge verified
[PASS] Scoreboard API Returns Solves & Scores - Scoreboard updated
[PASS] Admin Manual Score Adjustment (+50 pts) - Status 200
[PASS] User Profile Page /profile - Status 200
[PASS] User Change Password - Status 200
[PASS] Admin Audit Log Records All Administrative Events - Found recorded actions
[PASS] Security: CSRF Guard Rejects Invalid CSRF Token - Got status 403
======================================================================
RESULTS: 44/44 tests PASSED (100.0%)
======================================================================
```

---

## 🔍 Feature Verification Matrix

### 1. Health & Infrastructure
- **Endpoint**: `GET /health`
- **Behavior**: Returns `{"db":"ok","status":"ok","uptime_seconds":...}` with status 200. Returns 503 if database connection fails.
- **Verification**: Tested via `curl` and automated runner; responded with HTTP 200.

### 2. Path-Based Admin Route Isolation
- **Endpoint**: `GET /admin/*` (unauthenticated or non-admin)
- **Behavior**: Returns `404 Not Found` with the standard 404 template rather than `401` or `403`. This prevents route enumeration and scanner profiling.
- **Verification**: Verified that anonymous visits to `/admin/dashboard` and `/admin/audit` return 404, identical to nonexistent routes.

### 3. CSRF Protection
- **Mechanism**:
  - Pre-session cookie `ferris_pre` for anonymous users.
  - Session-bound token `ferris_session` for authenticated users.
  - Form validation on all POST/PUT/DELETE requests.
- **Verification**: Requests with absent or tampered tokens receive `403 Forbidden` (`Invalid or missing CSRF token`).

### 4. Rate Limiting (Sliding Window)
- **Configuration**:
  - User Login: 10 attempts / 60 seconds
  - User Registration: 10 attempts / 60 seconds
  - Admin Login: 3 attempts / 300 seconds
  - Team Join Preview: 10 attempts / 60 seconds
  - Flag Submission: 20 attempts / 60 seconds
- **Verification**: Tested admin login throttling; the 4th consecutive attempt within 5 minutes triggered `429 Too Many Requests`.

### 5. Admin & Operator Workflows
- **Bootstrap Credentials**: Defined in `.env` (`ADMIN_USERNAME`, `ADMIN_PASSWORD`). On boot, creates the root operator account (`is_admin=1`, `is_ferris=1`).
- **Dashboard (`/admin/dashboard`)**: Displays live team counts, user registrations, total solves, active round status, and recent audit logs.
- **Rounds Management (`/admin/rounds`)**:
  - Event Start (`POST /admin/event/start`): Records contest start timestamp and transitions state.
  - Round Transitions (`POST /admin/rounds/start`, `POST /admin/rounds/end`): Controls round limits and auto-triggers challenge pool distribution.
- **Challenge Authoring (`/admin/challenges`)**:
  - Multipart challenge upload: accepts title, category, difficulty tier, points, flag, and ZIP archive.
  - Constant-time HMAC flag hashing with per-challenge salt.
  - Hint creation with configurable point costs.
- **Team Management (`/admin/teams`)**:
  - Locking and unlocking teams.
  - Disbanding teams.
  - Manual score adjustment with logged audit reasons.
- **CSV Data Exports**:
  - `GET /admin/export/submissions`: Streams full submission records.
  - `GET /admin/export/solves`: Streams verified solves with points awarded.
- **Audit Logs (`/admin/audit`)**: Every administrative event (rounds, challenges, lock/unlock, manual score adjustments) is recorded in SQLite and rendered in the admin audit log.

### 6. Team & Player Experience
- **Registration (`/register`)**: Case-insensitive college ID normalization, password policy enforcement (minimum 8 chars), and instant session creation.
- **Onboarding (`/onboarding`)**:
  - Room creation generates an 8-character Crockford Base32 join code (e.g. `V9EHFASX`).
  - Room join via join code with preview functionality.
- **Roster Enforcement**: Teams are capped at 2 members (`TEAM_MAX_SIZE`). Automatically locks once complete.
- **Challenges (`/challenges`)**:
  - Teams only see challenges allocated to their assigned round pool (60% Easy, 30% Medium, 10% Hard).
  - Teams cannot access challenges unless locked.
- **Flag Verification (`/challenges/:id/submit`)**:
  - Trims whitespace, validates flag format.
  - Computes constant-time salted HMAC.
  - Incorrect attempts incur repeat penalty points.
  - Correct flag awards points, marks solve status, and invalidates scoreboard cache.
- **Leaderboard (`/scoreboard`, `/api/scoreboard`)**:
  - Ranks teams by total points descending.
  - Tie-breaking resolved by earliest last solve timestamp.
  - In-memory cached JSON endpoint for polling.
- **User Profile (`/profile`)**:
  - Displays user stats, team role, and total solves.
  - Password change hashes new password and revokes all other active sessions.

---

## 🛠️ Issues Identified & Fixed

During end-to-end testing, two issues were diagnosed and resolved:

### 1. `auth_svc::change_password` Argument Order Transposition
- **Location**: [`src/routes/profile.rs`](file:///home/arunhariharan/Projects/Nimbus2000/src/routes/profile.rs#L56-L62)
- **Problem**: The handler passed arguments as `(&state, user_id, current_password, new_password, session_id)`, but the service definition expected `(&state, user_id, session_id, current_password, new_password)`. Because all were string references, it compiled but failed at runtime with `"Current password is incorrect"`.
- **Fix**: Adjusted the invocation order to match `src/services/auth.rs`. Validated with successful test suite pass.

### 2. Multipart Challenge Form CSRF Token Transport
- **Location**: [`templates/admin/challenges.html`](file:///home/arunhariharan/Projects/Nimbus2000/templates/admin/challenges.html#L16)
- **Problem**: Standard HTML multipart forms do not include form fields in the pre-parsed urlencoded buffer. The CSRF guard expects multipart requests to provide tokens via the `x-csrf-token` header or the URL query string.
- **Fix**: Added `?csrf_token={{ csrf_token }}` to the form `action` URL attribute, allowing browser form submissions to pass CSRF validation.

---

## 📖 Manual Testing Runbook

Follow these steps to manually verify the platform in a web browser:

### Step 1: Start the Platform
```bash
cargo run
```
Open [http://localhost:8080](http://localhost:8080) in your browser.

### Step 2: Operator / Admin Login
1. Navigate to [http://localhost:8080/admin/login](http://localhost:8080/admin/login).
2. Enter the bootstrap credentials from `.env`:
   - **Username**: `PES2UG24AM126`
   - **Password**: `9c4abc0b6b657d47b3d565adc07f81c7`
3. Click **Sign In**.
4. Confirm redirection to `/admin/dashboard` showing system metrics.

### Step 3: Event & Challenge Setup
1. In the top navigation, click **Rounds**.
2. Click **Start Event** and set round duration (e.g. 60 minutes).
3. In the top navigation, click **Challenges**.
4. Fill in the **Upload New Challenge** form:
   - **Title**: `Sanity Check`
   - **Category**: `Misc`
   - **Difficulty**: `easy`
   - **Points**: `100`
   - **Flag**: `FLAG{welcome_to_ferris_ctf}`
   - **Description**: `Verify your submission pipeline.`
5. Click **Upload Challenge** and confirm it appears in the list.
6. Click **Add Hint** on the challenge:
   - **Content**: `The flag is literally in the description.`
   - **Cost**: `10` points.

### Step 4: Player Registration & Team Formation
1. Open an incognito / private browser window.
2. Navigate to [http://localhost:8080/register](http://localhost:8080/register).
3. Register Captain account:
   - **College ID / Username**: `player1`
   - **Display Name**: `Alice`
   - **Password**: `Password1234!`
4. On the onboarding page (`/onboarding`), enter a team name (`Team Red`) and click **Create Room**.
5. Copy the 8-character invite code displayed on the team dashboard (e.g. `AB12CD34`).
6. In a separate private window, register a second user (`player2` / `Bob`).
7. On the onboarding page, enter the invite code and click **Join Room**.
8. Confirm both Alice and Bob are listed on the team roster.

### Step 5: Challenge Solving & Scoring
1. In Alice's window, navigate to [http://localhost:8080/challenges](http://localhost:8080/challenges).
2. Click on the `Sanity Check` challenge card.
3. Test an incorrect flag: enter `FLAG{wrong_flag}`. Confirm rejection and repeat penalty logging.
4. Test hint unlocking: click **Unlock Hint** and confirm point deduction.
5. Submit the correct flag: `FLAG{welcome_to_ferris_ctf}`.
6. Confirm the green **Solved** badge appears.
7. Navigate to [http://localhost:8080/scoreboard](http://localhost:8080/scoreboard) and confirm `Team Red` appears on the leaderboard with their updated score.

### Step 6: Admin Management & CSV Export
1. Return to the admin session window.
2. Click **Teams** in the navigation. Locate `Team Red` and click **+Score** to award 50 bonus points.
3. In the top navigation, click **Audit Log** and verify all events are recorded with admin timestamps.
4. Test CSV export: download [http://localhost:8080/admin/export/solves](http://localhost:8080/admin/export/solves) and verify CSV contents.
