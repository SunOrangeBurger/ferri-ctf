# FerrisCTF Technical Specification v5

*Rust-based high-performance CTF platform. Corrections applied are listed in the Revision Notes at the end (v2 corrections, then v3 and v5 changes).*

## 1. System Overview and Architecture

FerrisCTF is a single-binary Rust CTF platform for 50 to 200 concurrent participants, 50+ challenges, and teams of 1 to 2 members (a single participant is also a team). It uses Axum and Tokio for async HTTP, embedded SQLite (WAL) for storage, MiniJinja for server-side rendering, and a room-based team system with an 8-character join code.

**Core design principles**

- Single statically linked binary. Templates and static assets are embedded at build time.
- All state in embedded SQLite (WAL mode).
- No plaintext flags in the DB or on the wire.
- Admin routes are indistinguishable from non-existent routes to non-admins, and Ferris-only routes are indistinguishable from non-existent routes to everyone except Ferris (sole exception: the admin login page, see 9.3).
- Team membership locked at event start.
- Randomized per-team challenge pools assigned each round (wave) in a 60/30/10 easy/medium/hard split. Solved challenges are never shown to that team again.

```text
+------------------------------------------+
|          Client (Browser / cURL)         |
+--------------------+---------------------+
                     | HTTP (polling for scoreboard)
                     v
+-------------------------------------------------------------------------+
| Rust Binary (Axum + Tokio)                                              |
|                                                                         |
|  Rate Limiter ---> Auth + RBAC Middleware ---> MiniJinja SSR            |
|  (tower-governor)  (ban check, admin guard)                             |
|                                                                         |
|  Challenge Pool Engine   HMAC-SHA256 Flag Verifier   Tower-HTTP Stream  |
|  Team Lifecycle Engine   Hint Service                CSRF Guard         |
+-------------------------------------------------------------------------+
        |                         |                        |
        v                         v                        v
  SQLite (WAL)            In-memory cache            ./uploads/
  embedded DB             (scoreboard TTL)           (*.zip files)
```

### 1.1 Roles

| Role | Where it exists | Capabilities |
| --- | --- | --- |
| Player | Web account | Belongs to one team. Views the team's current round pool, downloads files, submits flags, requests hints. |
| Captain | Web account (a player) | Player capabilities plus pre-lock team management: kick, transfer, rename (sets the team's leaderboard name), disband. Exactly one captain per team. |
| Admin | Web account (`is_admin = 1`) | Works with participants and scores only: manages teams and users (force actions, bans), manually adjusts scores (audit-logged), exports, and the audit log. Does not manage rounds, admins, challenges, or hints. |
| Ferris | Host machine and web app, above Admin | The person running the binary on their local machine. Sits above all else, with complete control over systems: can kick and add admins, players, captains, change scores, exact penalties, and everything else, plus the process, environment, database file, uploads, and logs. Ferris alone starts and ends rounds, sets round time limits, adds and removes admins, and manages challenges and hints. |

**Ferris** is the operator. Ferris sits above all else, they have complete control over systems, can kick and add admins, players, captains, change scores, exact penalities, everything a god in his own universe can accomplish. Ferris is also responsible for:

- Supplying the environment (`SERVER_SECRET`, `ADMIN_USERNAME`, `ADMIN_PASSWORD`, `DATABASE_URL`) and starting or stopping the binary.
- Network setup, the host firewall, and keeping the machine awake and powered during the event.
- Backups of `ferrisctf.db`, `./uploads/`, and `SERVER_SECRET`.
- Reading logs and acting on `/health`.
- Starting each round, ending each round, and optionally setting a time limit that ends a round automatically.
- Adding and removing admins.

Ferris has a web identity: the bootstrap account created from `ADMIN_USERNAME` / `ADMIN_PASSWORD` (see 9.8) is flagged `is_ferris = 1` (and `is_admin = 1`). Ferris-only routes follow the same 404 rule as admin routes (see 9.3).

Because Ferris can read the database file and `SERVER_SECRET`, Ferris is fully trusted and could in principle brute-force flags offline. Ferris should not compete as a participant.

### 1.2 Deployment Environment

The event is hosted on the **college Wi-Fi**, from Ferris's **Fedora desktop**, which runs the single binary directly. There is no cloud host.

- **Binding:** listen on `0.0.0.0:8080` so Wi-Fi clients can reach it. Participants use the desktop's LAN IP (or an mDNS name such as `hostname.local`).
- **Firewall:** Fedora's `firewalld` blocks the port by default. Open it, for example `sudo firewall-cmd --add-port=8080/tcp`, and confirm from a second device.
- **SELinux:** keep it enforcing. Run the binary from a path and user that SELinux permits, and make sure `./uploads/` and the DB file are writable by that user.
- **Address stability:** ask IT for a DHCP reservation for the desktop, or publish the IP on the day. A changed IP mid-event breaks every participant.
- **Client isolation:** many campus access points isolate clients from each other. Test from a phone on the same Wi-Fi well before the event, and ask IT to allow client-to-host traffic if it fails.
- **Plain HTTP:** without a TLS proxy the site runs over HTTP. The `Secure` cookie flag and HSTS must be disabled in that case (they apply only over HTTPS), or login will fail. Caddy with an internal certificate is optional but would warn browsers about trust.
- **Rate limiting:** if the college NATs Wi-Fi clients behind one address, per-IP limits would throttle everyone together. Check what address the server sees, and raise per-IP limits or trust a forwarded header if needed.
- **Host stability:** disable suspend and sleep on the desktop, keep it on mains power, and run the binary under a `systemd` unit so it restarts on failure.

## 2. Technology Stack and Dependencies

```toml
[dependencies]
# Web and async runtime
tokio = { version = "1.38", features = ["full"] }
axum = { version = "0.7", features = ["multipart"] }
tower = { version = "0.4", features = ["util"] }
tower-http = { version = "0.5", features = ["fs", "trace", "cors", "compression-gzip", "set-header", "limit"] }
tower-governor = "0.4"

# Database
sqlx = { version = "0.7", features = ["runtime-tokio", "sqlite", "migrate", "macros", "chrono", "uuid"] }

# Templates
minijinja = { version = "2.0", features = ["loader"] }

# Cryptography and security
argon2 = "0.5"      # password hashing
hmac = "0.12"       # flag HMAC + session signing
sha2 = "0.10"
subtle = "2.5"      # constant-time comparison
rand = "0.8"
hex = "0.4"
uuid = { version = "1.8", features = ["v4", "serde"] }
zeroize = "1.7"

# Serialization and utilities
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
chrono = { version = "0.4", features = ["serde"] }
dotenvy = "0.15"
thiserror = "1.0"
num_cpus = "1.16"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tokio-util = { version = "0.7", features = ["io"] }
axum-extra = { version = "0.9", features = ["cookie"] }
base64 = "0.22"
```

## 3. Configuration Constants

Defined in `config.rs`, overridable via environment variables.

| Constant | Default | Description |
| --- | --- | --- |
| POOL\_SIZE\_PER\_TEAM | 15 | Challenges assigned to each team per round. Derived as `PLANNED_QUESTIONS / ROUND_COUNT` (rounded to nearest) unless overridden |
| PLANNED\_QUESTIONS | 60 | Planned size of the question bank. Subject to change |
| ROUND\_COUNT | 4 | Planned number of rounds (waves). Subject to change |
| DIFFICULTY\_SPLIT | 60/30/10 | Easy/medium/hard share of each pool, in percent |
| REPEAT\_PENALTY\_POINTS | 10 | Points deducted from a challenge per re-appearance after the first |
| TEAM\_MIN\_SIZE | 1 | Minimum members for a valid team (a single participant is a team) |
| TEAM\_MAX\_SIZE | 2 | Maximum members per team |
| GRACE\_PERIOD\_MINUTES | 30 | Time after start for free agents to form a valid team |
| MAX\_UPLOAD\_BYTES | 31457280 | 30 MB zip limit |
| SESSION\_TTL\_HOURS | 24 | Cookie expiry |
| SCOREBOARD\_CACHE\_TTL\_SECONDS | 30 | In-memory cache TTL |
| JOIN\_CODE\_LENGTH | 8 | Crockford Base32 (no ambiguous characters) |
| SERVER\_SECRET | env | HMAC key for flags and sessions. Minimum 32 bytes |
| ADMIN\_USERNAME | env | First-admin bootstrap (college ID). This account is also the Ferris account |
| ADMIN\_PASSWORD | env | First-admin bootstrap password |

> **Warning:** SERVER\_SECRET is the HMAC key for every stored flag. Changing it after challenges are created invalidates all flags. Treat it as immutable once set, and back it up.

## 4. Database Schema

Pragmas must apply on every new connection (via `SqliteConnectOptions` or `after_connect` in `db.rs`):

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA busy_timeout = 5000;
PRAGMA foreign_keys = ON;
```

Migration `migrations/0001_init.sql`:

```sql
-- Individual accounts
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,                    -- UUIDv4
    username TEXT UNIQUE NOT NULL,          -- college ID
    display_name TEXT NOT NULL,
    password_hash TEXT NOT NULL,            -- Argon2id
    is_admin INTEGER NOT NULL DEFAULT 0,
    is_ferris INTEGER NOT NULL DEFAULT 0,   -- operator account; set only by bootstrap, never by routes
    is_banned INTEGER NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Single-row event state
CREATE TABLE IF NOT EXISTS event_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    scheduled_start DATETIME,               -- optional configured start
    started_at DATETIME                     -- NULL until event starts (Round 1 start)
);
INSERT OR IGNORE INTO event_state (id) VALUES (1);

-- Rounds (waves). Only Ferris creates, ends, and sets limits on rounds.
CREATE TABLE IF NOT EXISTS rounds (
    round_number INTEGER PRIMARY KEY,       -- 1, 2, 3, ...
    started_at DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f', 'now')),
    time_limit_minutes INTEGER CHECK (time_limit_minutes IS NULL OR time_limit_minutes > 0),
    auto_end_at DATETIME,                   -- started_at + time limit; NULL = no limit
    ended_at DATETIME                       -- NULL while the round is active
);
-- At most one round may be active (ended_at IS NULL) at a time; enforced in a BEGIN IMMEDIATE transaction.

-- Teams (rooms)
CREATE TABLE IF NOT EXISTS teams (
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,              -- leaderboard display name, chosen by the captain
    join_code TEXT UNIQUE NOT NULL,         -- 8 chars, Crockford Base32
    captain_id TEXT NOT NULL,
    is_locked INTEGER NOT NULL DEFAULT 0,
    disbanded INTEGER NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(captain_id) REFERENCES users(id) ON DELETE RESTRICT
);

-- Team membership
CREATE TABLE IF NOT EXISTS team_members (
    team_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'member',    -- 'captain' | 'member'
    joined_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (team_id, user_id),
    FOREIGN KEY(team_id) REFERENCES teams(id) ON DELETE CASCADE,
    FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
);
-- Enforces exactly one team per user
CREATE UNIQUE INDEX IF NOT EXISTS idx_team_members_one_team ON team_members(user_id);
-- Enforces at most one captain per team
CREATE UNIQUE INDEX IF NOT EXISTS idx_team_members_one_captain ON team_members(team_id) WHERE role = 'captain';

-- Challenges
CREATE TABLE IF NOT EXISTS challenges (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    category TEXT NOT NULL,                 -- Web | Pwn | Crypto | Forensics | Rev | Misc
    difficulty TEXT NOT NULL CHECK (difficulty IN ('easy', 'medium', 'hard')),  -- fixed at creation time, never edited
    points INTEGER NOT NULL CHECK (points >= 0),
    flag_hmac TEXT NOT NULL,                -- hex(HMAC-SHA256(SERVER_SECRET, salt || flag))
    flag_salt TEXT NOT NULL,                -- 32 hex chars (16 bytes)
    file_path TEXT,
    file_name TEXT,
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Hints
CREATE TABLE IF NOT EXISTS hints (
    id TEXT PRIMARY KEY,
    challenge_id TEXT NOT NULL,
    content TEXT NOT NULL,
    point_cost INTEGER NOT NULL DEFAULT 0 CHECK (point_cost >= 0),
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(challenge_id) REFERENCES challenges(id) ON DELETE CASCADE
);

-- Hint usage per team
CREATE TABLE IF NOT EXISTS hint_usage (
    team_id TEXT NOT NULL,
    hint_id TEXT NOT NULL,
    used_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (team_id, hint_id),
    FOREIGN KEY(team_id) REFERENCES teams(id) ON DELETE CASCADE,
    FOREIGN KEY(hint_id) REFERENCES hints(id) ON DELETE CASCADE
);

-- Per-team, per-round challenge pool. One row per appearance of a challenge in a team's round pool.
-- The number of earlier rows for the same (team, challenge) is the number of earlier appearances.
CREATE TABLE IF NOT EXISTS team_challenge_pool (
    team_id TEXT NOT NULL,
    round_number INTEGER NOT NULL,
    challenge_id TEXT NOT NULL,
    repeat_penalty INTEGER NOT NULL DEFAULT 0 CHECK (repeat_penalty >= 0),  -- REPEAT_PENALTY_POINTS * earlier appearances, frozen at assignment
    assigned_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (team_id, round_number, challenge_id),
    FOREIGN KEY(team_id) REFERENCES teams(id) ON DELETE CASCADE,
    FOREIGN KEY(round_number) REFERENCES rounds(round_number) ON DELETE CASCADE,
    FOREIGN KEY(challenge_id) REFERENCES challenges(id) ON DELETE CASCADE
);

-- Submissions (no plaintext flags stored)
CREATE TABLE IF NOT EXISTS submissions (
    id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    challenge_id TEXT NOT NULL,
    is_correct INTEGER NOT NULL,
    submitted_hash TEXT NOT NULL,           -- HMAC of submitted flag (audit only)
    submitted_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(team_id) REFERENCES teams(id) ON DELETE CASCADE,
    FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY(challenge_id) REFERENCES challenges(id) ON DELETE CASCADE
);

-- Solves (millisecond timestamp for tie-breaking)
CREATE TABLE IF NOT EXISTS solves (
    team_id TEXT NOT NULL,
    challenge_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    round_number INTEGER NOT NULL,          -- round in which the solve happened
    points_awarded INTEGER NOT NULL,
    solved_at DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f', 'now')),
    PRIMARY KEY (team_id, challenge_id),
    FOREIGN KEY(team_id) REFERENCES teams(id) ON DELETE CASCADE,
    FOREIGN KEY(challenge_id) REFERENCES challenges(id) ON DELETE CASCADE,
    FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY(round_number) REFERENCES rounds(round_number) ON DELETE CASCADE
);

-- Manual score adjustments by admins or Ferris (each also written to audit_log)
CREATE TABLE IF NOT EXISTS score_adjustments (
    id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL,
    admin_id TEXT NOT NULL,
    delta INTEGER NOT NULL,                 -- positive or negative
    reason TEXT NOT NULL,
    created_at DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f', 'now')),
    FOREIGN KEY(team_id) REFERENCES teams(id) ON DELETE CASCADE,
    FOREIGN KEY(admin_id) REFERENCES users(id) ON DELETE RESTRICT
);

-- Admin audit log (admin rows cannot be deleted while logs exist)
CREATE TABLE IF NOT EXISTS audit_log (
    id TEXT PRIMARY KEY,
    admin_id TEXT NOT NULL,
    action TEXT NOT NULL,
    target_type TEXT,
    target_id TEXT,
    details TEXT,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(admin_id) REFERENCES users(id) ON DELETE RESTRICT
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_solves_team ON solves(team_id);
CREATE INDEX IF NOT EXISTS idx_solves_challenge ON solves(challenge_id);
CREATE INDEX IF NOT EXISTS idx_submissions_team_chal ON submissions(team_id, challenge_id);
CREATE INDEX IF NOT EXISTS idx_pool_team ON team_challenge_pool(team_id);
CREATE INDEX IF NOT EXISTS idx_pool_team_round ON team_challenge_pool(team_id, round_number);
CREATE INDEX IF NOT EXISTS idx_challenges_active ON challenges(is_active);
CREATE INDEX IF NOT EXISTS idx_challenges_difficulty ON challenges(difficulty, is_active);
CREATE INDEX IF NOT EXISTS idx_score_adj_team ON score_adjustments(team_id);
```

Notes: `team_members.role` and `teams.captain_id` must be updated together in one transaction. Prefer deactivating a challenge over deleting it, because deletion cascades to solves and changes scores. A challenge's `difficulty` is assigned at creation and is not editable, because it drives pool composition and tie-breaking.

## 5. Team Lifecycle

### 5.1 Registration and Onboarding

1. User registers with college ID (username), display name, and password.
2. User is redirected to the onboarding wizard:
   - **Create a room:** enter team name. The captain chooses this name; it is what appears on the leaderboard for the team. The system generates an 8-character `join_code` (Crockford Base32 alphabet: `0123456789ABCDEFGHJKMNPQRSTVWXYZ`). Creator becomes captain. A creator who never shares the code is a valid single-member team.
   - **Join a room:** enter `join_code`. The system shows current members (username and display name) for verification before confirming.
3. A user is in exactly one team at a time, enforced by the unique index on `team_members(user_id)` and by application logic. A team has exactly one captain, enforced by the partial unique index on `team_members(team_id) WHERE role = 'captain'`.
4. Join checks (size, lock state) run inside a write transaction so concurrent joins cannot exceed TEAM\_MAX\_SIZE.

### 5.2 Pre-Event (before questions revealed)

- Members may leave voluntarily. The captain cannot leave while another member remains: they must transfer captaincy first, or disband.
- Captain may kick members, transfer captaincy, rename the team (unique name), or disband the team (`disbanded = 1`, all `team_members` rows removed, members become free agents).
- Team size must stay at or below 2. Joining is rejected if full.

### 5.3 Event Start (questions revealed)

- Ferris triggers Start Event (or the configured `scheduled_start` is reached). `event_state.started_at` is set and Round 1 starts (see 6.4).
- All teams with at least TEAM\_MIN\_SIZE (1) members are locked (`is_locked = 1`) and receive their pool for the round.
- **Grace period:** until `started_at + GRACE_PERIOD_MINUTES`, free agents may create or join unlocked teams. A team locks (and receives its pool for the active round) the moment it reaches TEAM\_MIN\_SIZE during this window, which for a newly created team is immediately. Locked teams cannot be joined. After the grace period, free agents cannot view challenges until they are in a valid locked team.
- Post-lock: no leaving, kicking, or joining. Only admin override.

### 5.4 Admin Overrides

Force-join or remove a user from any team, disband any team, change captain, lock or unlock a team, ban or unban a user, and manually adjust a team's score (see 7.4). All actions are written to `audit_log`.

## 6. Challenge Pool and Flag Verification

### 6.1 Pool Assignment

Pools are assigned per round (wave). Each team, including single-member teams, receives its own independently randomized pool when a round starts.

- **Size:** POOL\_SIZE\_PER\_TEAM challenges per team per round. The default follows the question bank and round plan: with 60 planned questions and 4 rounds, 15 per round. If the figures change, the size changes with them.
- **Difficulty split:** 60% easy, 30% medium, 10% hard. When the percentages do not divide evenly, round to the nearest distribution using largest remainder, with ties going to the more common tier. For a pool of 15 this gives 9 easy, 5 medium, 1 hard.
- **Source:** each round, the pool is redistributed from the full question bank of active challenges, excluding every challenge the team has already solved. A challenge the team was shown earlier but did not solve may be drawn again.
- **Randomization:** each team's draw is independent, to reduce flag sharing between teams. Overlap between teams is allowed, and flag sharing cannot be fully eliminated.
- **Never again once solved:** a team never sees a challenge it has solved in any later round.
- **Repeat penalty:** each re-appearance of an unsolved challenge costs REPEAT\_PENALTY\_POINTS (10) points. The penalty is `10 × (earlier appearances of that challenge in this team's pools)`, frozen into `team_challenge_pool.repeat_penalty` at assignment. The points shown to the team are `max(0, challenge.points - repeat_penalty)`.
- **No mid-round replenishment:** solving a challenge does not add a new one. New challenges arrive at the next round start.
- If a difficulty tier has fewer eligible challenges than its quota, the team receives all that remain in that tier and the pool shrinks. The shortfall is not backfilled from other tiers.
- Deactivated challenges are hidden from pool views and are not drawn until reactivated.
- A team that locks late (grace period) is assigned its pool for the active round at that moment. If no round is active, it receives its pool at the next round start.
- All assignments run in a `BEGIN IMMEDIATE` transaction to prevent over-assignment under concurrency.

### 6.2 Flag Storage and Verification

Flags are never stored in plaintext and never serialized to the client.

- `flag_salt` = 16 random bytes, hex-encoded.
- `flag_hmac` = hex(HMAC-SHA256(SERVER\_SECRET, salt || flag)).
- Flags are compared case-sensitively after trimming surrounding whitespace.

```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

fn compute(secret: &[u8], salt: &str, flag: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("any key length");
    mac.update(salt.as_bytes());
    mac.update(flag.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub fn verify_flag(secret: &[u8], salt: &str, submitted: &str, stored_hmac: &str) -> bool {
    let trimmed = Zeroizing::new(submitted.trim().to_owned());
    let computed = compute(secret, salt, &trimmed);
    computed.as_bytes().ct_eq(stored_hmac.as_bytes()).into()
}
```

Submitted flags are recorded as `HMAC-SHA256(SERVER_SECRET, submitted)` in `submissions.submitted_hash`, for audit only.

Submission preconditions (checked before verification): a round is active and has not passed its `auto_end_at`, team is locked, challenge is active and in the team's pool for the active round, challenge is not already solved by the team, user is not banned. Submissions outside an active round are rejected.

### 6.3 Hint Scoring

- Ferris defines hints per challenge with `point_cost` (see Section 8).
- Player requests a hint from the challenge page. A confirmation modal shows the cost. On confirm, content is revealed and `hint_usage` is recorded for the whole team (any member's request spends for the team).
- On solve: `points_awarded = max(0, challenge.points - repeat_penalty - sum(hint_costs_used_by_team))`, where `repeat_penalty` is the frozen value for the team's current-round pool row. Points never go below zero.
- Hints cannot be requested after the challenge is solved, or for challenges outside the team's current round pool.

### 6.4 Rounds (Waves)

- Only Ferris can start a round, end a round, and set or change a round's time limit. Admins cannot.
- ROUND\_COUNT only sizes the per-round pool (6.1). It does not cap the number of rounds: Ferris decides when to start another round and when the event is over.
- **Start:** Ferris starts round N. Round N-1 must have ended, and only one round is active at a time. Starting a round inserts the `rounds` row, assigns a new pool to every locked team (6.1), and invalidates the scoreboard cache. Starting Round 1 is the event start (5.3).
- **Time limit:** Ferris may set a limit in minutes, at start or while the round runs. This sets `auto_end_at = started_at + limit`. Ferris may also change or clear it while the round is active, but not to a time already past.
- **End:** Ferris ends a round manually, or it ends automatically when `auto_end_at` is reached. A background Tokio task checks `auto_end_at` every second against the DB, so the timer survives a restart. Submission checks also compare the current time with `auto_end_at`, so no solve is accepted after the limit even if the task has not yet run.
- **Between rounds:** challenge pages, downloads, submissions, and hints are closed for players. The scoreboard stays available.
- Starting, ending, and limit changes are written to `audit_log`.

## 7. Scoreboard and Tie-Breaking

### 7.1 Ranking Order

Teams are ranked by the following, in order. Steps 2 to 5 are applied only between teams whose total scores are identical:

1. **Total score**, higher first. Total score is the sum of `solves.points_awarded` plus the sum of `score_adjustments.delta`.
2. **Countback by difficulty.** Compare the number of challenges solved in each tier, hardest first: hard, then medium, then easy. The first tier where counts differ decides, and more solves wins. Difficulty is the value assigned at challenge creation. Example: if two teams are tied on score and Team A solved 4 easy while Team B solved 1 hard, Team B ranks higher.
3. **Countback by solve counts.** If all three tier counts are equal, compare the cumulative solve counts of the challenges each team solved. A challenge's solve count is the number of teams that solved it across all rounds. For each tier in turn (hard, then medium, then easy), sum the solve counts of the team's solved challenges in that tier. The lower sum wins, because the team solved the harder-in-practice challenges. Example: both teams solved 3 hard, 10 medium and 20 easy. Team A's hard challenges have 5, 10 and 15 solves in total (sum 30). Team B's have 6, 10 and 12 (sum 28). Team B wins this countback.
4. **Time to score.** If countback is also tied, the team that first reached its final tied score earlier ranks higher (see 7.3).
5. **Team creation time**, earlier first (final fallback).

### 7.2 Query

Steps 1 to 3 are computed in SQL. Steps 4 and 5 are resolved in the scoring service (`services/scoring.rs`) for teams still tied.

```sql
WITH solve_counts AS (
    SELECT challenge_id, COUNT(*) AS n_solves
    FROM solves
    GROUP BY challenge_id
),
adj AS (
    SELECT team_id, SUM(delta) AS adj_total
    FROM score_adjustments
    GROUP BY team_id
),
agg AS (
    SELECT
        s.team_id,
        SUM(s.points_awarded) AS solve_points,
        SUM(CASE WHEN c.difficulty = 'hard'   THEN 1 ELSE 0 END) AS hard_solved,
        SUM(CASE WHEN c.difficulty = 'medium' THEN 1 ELSE 0 END) AS medium_solved,
        SUM(CASE WHEN c.difficulty = 'easy'   THEN 1 ELSE 0 END) AS easy_solved,
        SUM(CASE WHEN c.difficulty = 'hard'   THEN sc.n_solves ELSE 0 END) AS hard_solve_sum,
        SUM(CASE WHEN c.difficulty = 'medium' THEN sc.n_solves ELSE 0 END) AS medium_solve_sum,
        SUM(CASE WHEN c.difficulty = 'easy'   THEN sc.n_solves ELSE 0 END) AS easy_solve_sum
    FROM solves s
    JOIN challenges c    ON c.id = s.challenge_id
    JOIN solve_counts sc ON sc.challenge_id = s.challenge_id
    GROUP BY s.team_id
)
SELECT
    t.id AS team_id,
    t.name AS team_name,
    t.created_at AS team_created_at,
    COALESCE(a.solve_points, 0) + COALESCE(adj.adj_total, 0) AS total_score,
    COALESCE(a.hard_solved, 0)       AS hard_solved,
    COALESCE(a.medium_solved, 0)     AS medium_solved,
    COALESCE(a.easy_solved, 0)       AS easy_solved,
    COALESCE(a.hard_solve_sum, 0)    AS hard_solve_sum,
    COALESCE(a.medium_solve_sum, 0)  AS medium_solve_sum,
    COALESCE(a.easy_solve_sum, 0)    AS easy_solve_sum
FROM teams t
LEFT JOIN agg a   ON a.team_id = t.id
LEFT JOIN adj     ON adj.team_id = t.id
WHERE t.disbanded = 0 AND t.is_locked = 1
ORDER BY
    total_score DESC,
    hard_solved DESC, medium_solved DESC, easy_solved DESC,
    hard_solve_sum ASC, medium_solve_sum ASC, easy_solve_sum ASC,
    t.created_at ASC;
```

The scoring service reorders rows that tie through all SQL keys using `reached_score_at` (7.3) before `team_created_at`. The leaderboard shows each team's name and its members' display names.

### 7.3 Time Tie-Break

`reached_score_at` is the timestamp at which the team first reached its final (current) total score and stayed there. The service replays the team's score events (solves by `solved_at`, adjustments by `created_at`) in order, keeping a running total, and takes the time of the earliest event after which the running total equals the final total and never changes again. A solve that awards zero points, or any event that leaves the total unchanged, does not move this timestamp. The earlier timestamp wins. All teams share the same round reveal times, so comparing timestamps is the same as comparing time elapsed since reveal.

### 7.4 Manual Score Adjustments

Admins (and Ferris) may adjust any team's score by a positive or negative `delta` with a required reason, via `POST /admin/teams/:id/score`. Each adjustment inserts a `score_adjustments` row and an `audit_log` entry (`action = 'score_adjust'`, with delta and reason in `details`). Adjustments count toward total score and the time tie-break (7.3), and invalidate the scoreboard cache.

### 7.5 Caching

- In-memory cache (`RwLock`) with TTL of SCOREBOARD\_CACHE\_TTL\_SECONDS (30).
- Invalidated on any new solve, team lock, disband, round start or end, score adjustment, or admin override.
- The page is rendered server-side via MiniJinja. Clients poll `GET /api/scoreboard` (JSON).

## 8. API and Route Matrix

| Method | Endpoint | Access | Description |
| --- | --- | --- | --- |
| GET | / | Public | Landing page |
| GET | /static/\* | Public | Embedded static assets |
| GET | /login | Public | Login page |
| POST | /login | Public | Login (rate-limited) |
| GET | /register | Public | Registration page |
| POST | /register | Public | Registration (rate-limited) |
| POST | /logout | Auth | Clears session cookie (CSRF-protected) |
| GET | /onboarding | Auth (no team) | Create or join a room |
| POST | /team/create | Auth (no team) | Create room |
| POST | /team/join | Auth (no team) | Join via code |
| GET | /team/preview/:code | Auth (no team) | Show members before joining (rate-limited with /team/join) |
| GET | /team | Player | Team dashboard |
| POST | /team/leave | Player (pre-lock) | Leave team |
| POST | /team/kick | Captain (pre-lock) | Kick member |
| POST | /team/transfer | Captain (pre-lock) | Transfer captaincy |
| POST | /team/rename | Captain (pre-lock) | Rename team |
| POST | /team/disband | Captain (pre-lock) | Disband team |
| GET | /challenges | Player (locked team, active round) | Pooled challenges for the current round |
| GET | /challenges/:id | Player (in pool) | Challenge detail |
| GET | /challenges/:id/download | Player (in pool) | Download zip (streamed) |
| POST | /challenges/:id/submit | Player (in pool) | Submit flag |
| POST | /challenges/:id/hint | Player (in pool) | Request hint |
| GET | /scoreboard | Public | Leaderboard page |
| GET | /api/scoreboard | Public | JSON scoreboard (polling) |
| GET | /profile | Auth | User profile |
| POST | /profile/password | Auth | Change password |
| GET | /admin/login | Public | Admin login page |
| POST | /admin/login | Public (rate-limited) | Admin login |
| GET | /admin/dashboard | Admin | Overview |
| GET | /admin/teams | Admin | Team management |
| POST | /admin/teams/:id/action | Admin | Force actions |
| POST | /admin/teams/:id/score | Admin | Manual score adjustment (audit-logged) |
| POST | /admin/users/:id/action | Admin | Ban, unban |
| GET | /admin/challenges | Ferris | Challenge list |
| POST | /admin/challenges/new | Ferris | Upload challenge (multipart) |
| POST | /admin/challenges/:id/edit | Ferris | Edit challenge |
| POST | /admin/challenges/:id/toggle | Ferris | Activate or deactivate |
| POST | /admin/challenges/bulk | Ferris | Bulk activate or deactivate |
| POST | /admin/challenges/:id/delete | Ferris | Delete challenge |
| POST | /admin/challenges/:id/hints | Ferris | Add hint |
| POST | /admin/hints/:id/edit | Ferris | Edit hint |
| POST | /admin/hints/:id/delete | Ferris | Delete hint |
| POST | /admin/event/start | Ferris | Lock teams, start event (starts Round 1) |
| GET | /admin/rounds | Ferris | Round status and controls |
| POST | /admin/rounds/start | Ferris | Start the next round (assigns pools) |
| POST | /admin/rounds/end | Ferris | End the active round |
| POST | /admin/rounds/limit | Ferris | Set, change, or clear the active round's time limit |
| GET | /admin/admins | Ferris | List admins |
| POST | /admin/admins/add | Ferris | Grant admin to an existing user |
| POST | /admin/admins/:id/remove | Ferris | Revoke admin from a user |
| GET | /admin/export/:kind | Admin | CSV export (submissions, solves) |
| GET | /admin/audit | Admin | Audit log |
| GET | /health | Public | Healthcheck |

## 9. Security Specification

### 9.1 Session Management

- HMAC-SHA256 signed cookie named `ferris_session`.
- Contents: `session_id | user_id | expiry | is_admin`. `session_id` is a random 128-bit value.
- Flags: HttpOnly, SameSite=Strict, Secure (when behind HTTPS), Path=/.
- TTL 24 hours. Rotated on login, cleared on logout.
- Revocation: an in-memory set of revoked `session_id` values. The set is lost on restart, so admin-sensitive checks (`is_admin`, `is_banned`) are re-verified against the DB on each request, and a restart without revocation persistence is an accepted limitation. `is_ferris` is never carried in the cookie and is always read from the DB.
- Signature verification uses constant-time comparison.

### 9.2 CSRF Protection

- All state-changing requests (every POST, including logout) carry a CSRF token: HMAC of session ID and nonce.
- Token validated server-side on every POST. State-changing actions are never exposed on GET.

### 9.3 Admin Route Protection

- All `/admin/*` routes require an admin session. Non-admin or unauthenticated access returns `404 Not Found` with the standard public layout. No redirects, no 403.
- Routes marked Ferris in Section 8 additionally require `is_ferris = 1`. An authenticated admin who is not Ferris receives the same 404.
- The sole exception is `/admin/login`, which must be reachable to log in. It is unlinked from the UI. Operators may relocate it via config if desired.
- Admin login failures are rate-limited to 3 attempts per IP per 5 minutes.
- Admin authenticates through the same Argon2id verification as users plus an `is_admin` check. There is no separate plaintext credential comparison.
- Login for unknown usernames performs a dummy Argon2 verification so response timing does not reveal account existence.

### 9.4 Rate Limits

| Route | Limit |
| --- | --- |
| /login, /register | 10/min/IP |
| /admin/login | 3/5min/IP |
| /challenges/:id/submit | 20/min/team |
| /team/join, /team/preview/:code | 10/min/IP (shared bucket) |
| /challenges/:id/download | 30/min/team |

Per-team limits require a custom `tower-governor` key extractor keyed on `team_id`; the default extractor keys on IP.

### 9.5 Argon2 DoS Protection

- A semaphore limits concurrent Argon2 operations to `num_cpus * 2`. Hashing runs in `spawn_blocking`.
- Requests beyond the limit queue briefly, then fail with 503.

### 9.6 File Upload

- Max size 30 MB, enforced during stream read (413 Payload Too Large). Axum's default body limit must be raised (`DefaultBodyLimit`) on the upload route to MAX\_UPLOAD\_BYTES plus multipart overhead.
- Magic bytes: first 4 bytes must be `50 4B 03 04` (`PK\x03\x04`).
- The stream is written to a temp file, validated, then atomically renamed to `./uploads/{challenge_uuid}.zip`.
- The stored path is derived server-side from the UUID, never from user input. The uploads directory is canonicalized, and any path escaping `./uploads/` is rejected.
- Original filename is sanitized for Content-Disposition (strip `"`, `\r`, `\n`, `;`, and path separators).

### 9.7 Security Headers (tower-http)

- `Content-Security-Policy: default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'`
- `X-Frame-Options: DENY`
- `Referrer-Policy: no-referrer`
- `Strict-Transport-Security: max-age=31536000; includeSubDomains` (HTTPS only)
- `X-Content-Type-Options: nosniff`

### 9.8 Admin Bootstrap

On first boot, if no admin exists:

1. Create the user from `ADMIN_USERNAME` / `ADMIN_PASSWORD` in the environment, with `is_admin = 1` and `is_ferris = 1`. This is the Ferris account.
2. Log a warning instructing the operator to change the password.
3. Never hardcode credentials.

Further admins are added and removed only by Ferris through `/admin/admins/*`. Granting sets `is_admin = 1` on an existing user, and removal sets it to 0 (the account and its audit rows remain). The Ferris account cannot be removed or demoted through the web app. Both actions are written to `audit_log`.

## 10. Streaming File Downloads

The path comes from the DB, the challenge must be active and in the team's pool for the active round, and the team must be locked.

```rust
async fn download_challenge_file(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<UserSession>,
    Path(challenge_id): Path<Uuid>,
) -> Result<Response, AppError> {
    let team_id = session.team_id.ok_or(AppError::NotFound("Not found".into()))?;
    let cid = challenge_id.to_string();

    // 1. Challenge must be active and in this team's pool for the active round
    let record = sqlx::query!(
        "SELECT c.file_path, c.file_name
         FROM challenges c
         JOIN team_challenge_pool p ON p.challenge_id = c.id
         JOIN teams t ON t.id = p.team_id
         JOIN rounds r ON r.round_number = p.round_number
         WHERE p.team_id = ? AND c.id = ? AND c.is_active = 1 AND t.is_locked = 1
           AND r.ended_at IS NULL",
        team_id,
        cid
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound("Not found".into()))?;

    let path = std::path::PathBuf::from(
        record.file_path.ok_or(AppError::NotFound("Not found".into()))?,
    );

    // 2. Stream file
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| AppError::NotFound("Not found".into()))?;
    let len = file.metadata().await.map(|m| m.len()).ok();
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(file));

    let safe_name = sanitize_header_filename(
        record.file_name.unwrap_or_else(|| "challenge.zip".into()),
    );
    let disposition = format!("attachment; filename=\"{}\"", safe_name);

    let mut resp = Response::new(body);
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_str(&disposition)?);
    if let Some(l) = len {
        h.insert(header::CONTENT_LENGTH, HeaderValue::from(l));
    }
    Ok(resp)
}
```

## 11. Quality-of-Life Features

**Players**

- Onboarding wizard (create or join room).
- Room member preview before joining.
- Team dashboard: invite code (copyable), member list with roles, pool progress, solve history, current rank.
- Current round indicator with a countdown when the round has a time limit.
- Challenge filters: category, status (unsolved / solved), points.
- Solved badges and per-category progress bars.
- Hint request UI with explicit cost confirmation.
- Toast notifications for solves, team events, and hint unlocks.
- Dark mode and responsive layout.
- Profile page with password change.

**Captains**

- Kick, transfer captaincy, rename, disband (all pre-lock).
- Pending join-request approval is **out of scope for this version** (no schema or routes defined). It may be added in a later version.

**Admins**

- Team management dashboard with search and filter.
- Force-join or remove users, change captain, disband teams.
- Manual score adjustments with required reason (audit-logged).
- Audit log viewer.
- CSV export of submissions and solves.

**Ferris**

- Start the event (Round 1), start and end rounds, set or change round time limits.
- Add and remove admins.
- Bulk challenge activation and deactivation.
- Challenge upload, edit and delete (including setting difficulty at creation).
- Hint management (create, edit, delete).

## 12. Project Structure

```text
FerrisCTF/
├── Cargo.toml
├── .env.example
├── migrations/
│   └── 0001_init.sql
├── static/                  (embedded at build time)
├── src/
│   ├── main.rs
│   ├── config.rs
│   ├── db.rs
│   ├── errors.rs
│   ├── session.rs
│   ├── csrf.rs
│   ├── templates.rs
│   ├── models/
│   │   ├── mod.rs
│   │   ├── user.rs
│   │   ├── team.rs
│   │   ├── challenge.rs
│   │   ├── hint.rs
│   │   └── round.rs
│   ├── routes/
│   │   ├── mod.rs
│   │   ├── auth.rs
│   │   ├── onboarding.rs
│   │   ├── team.rs
│   │   ├── challenges.rs
│   │   ├── scoreboard.rs
│   │   ├── profile.rs
│   │   └── admin.rs
│   ├── services/
│   │   ├── pool.rs
│   │   ├── rounds.rs
│   │   ├── scoring.rs
│   │   ├── verifier.rs
│   │   ├── files.rs
│   │   ├── hints.rs
│   │   └── team.rs
│   └── middleware/
│       ├── auth.rs
│       ├── admin_guard.rs
│       └── rate_limit.rs
├── templates/
│   ├── base.html
│   ├── index.html
│   ├── auth/            (login.html, register.html)
│   ├── onboarding/      (index.html, preview.html)
│   ├── team/            (dashboard.html)
│   ├── challenges/      (list.html, detail.html)
│   ├── scoreboard.html
│   ├── profile.html
│   ├── errors/          (404.html, 500.html)
│   └── admin/           (login, dashboard, teams, challenges, challenge_edit, audit, rounds, admins)
└── uploads/             (gitignored)
```

## 13. Build Instructions (Ordered)

1. **Workspace initialization.** Generate `Cargo.toml` with the dependencies in Section 2, create the structure in Section 12, and add `.env.example` with `SERVER_SECRET`, `ADMIN_USERNAME`, `ADMIN_PASSWORD`, `DATABASE_URL`.
2. **Database engine (`db.rs`).** Use `SqlitePoolOptions::new().max_connections(16)` (SQLite has a single writer, so a large pool adds contention, not throughput). Apply all four PRAGMAs on every connection. Use `BEGIN IMMEDIATE` for write transactions. Embed migrations with `sqlx::migrate!()`.
3. **Config (`config.rs`).** Load env vars via dotenvy and define the Section 3 constants. Fail fast if `SERVER_SECRET` (under 32 bytes) or admin bootstrap vars are missing on first run.
4. **Models.** `user.rs`, `team.rs`, `challenge.rs`, `hint.rs`, `round.rs` with `sqlx::FromRow`.
5. **Session and CSRF.** Signed cookie issuance and verification, revocation set, CSRF token generation and validation.
6. **Team service.** `create_team`, `join_team`, `leave_team`, `kick_member`, `transfer_captain`, `rename_team`, `disband_team`, `preview_team_by_code`, and `lock_all_teams` (called at event start; locks every team with at least TEAM\_MIN\_SIZE members). Grace-period joins lock a team as soon as it is valid.
7. **Pool and round services.** `assign_round_pool(pool, team_id, round_number)` implements 6.1: difficulty quotas by largest remainder, exclusion of solved challenges, random per-team draw, and frozen repeat penalties, all in a `BEGIN IMMEDIATE` transaction. `rounds.rs` implements `start_round`, `end_round`, `set_time_limit`, and the background auto-end task (6.4).
8. **Verifier service.** `hash_flag(plain, salt)` and `verify_flag(plain, salt, stored_hmac)` using HMAC-SHA256 and `subtle::ConstantTimeEq`, with zeroized input buffers.
9. **Hint and scoring services.** `request_hint(team_id, hint_id)` and `compute_awarded_points(challenge, team_id) -> i64` (applies repeat penalty and hint costs, floored at zero). `scoring.rs` builds the ranking in 7.1 to 7.3 and applies score adjustments.
10. **File service.** `validate_and_store_zip(multipart_stream) -> Result<(PathBuf, String)>` enforcing 30 MB, magic bytes, sanitized names.
11. **Routes.** Implement all routes in Section 8. Wire middleware: auth (including ban check), admin\_guard (returns 404, with a stricter Ferris guard for Ferris routes), rate\_limit, CSRF.
12. **Templates.** MiniJinja environment in `templates.rs`, all templates inheriting from `base.html`, embedded in the binary for release builds.
13. **Main.** Initialize tracing, config, DB, and router. Bootstrap the admin and Ferris account if none exists. Spawn the round auto-end task. Bind to `0.0.0.0:8080` (configurable). Graceful shutdown on SIGINT/SIGTERM.
14. **Tests.** Unit tests for verifier, pool (quotas, exclusion of solved, repeat penalty floor), scoring (countback and time tie-breaks), rounds, and team services. Integration test: register, create team, join, start event, solve, scoreboard, end round, start next round.

## 14. Operational Notes

- **Backup:** `sqlite3 ferrisctf.db ".backup backup.db"`, plus an archive of `./uploads/`. Also back up `SERVER_SECRET` separately.
- **Reverse proxy:** run behind nginx or Caddy with TLS termination. Set the Secure cookie flag when `X-Forwarded-Proto: https`, and trust that header only from the proxy. Configure the proxy's real client IP header for rate limiting.
- **Monitoring:** `GET /health` returns `{ "status": "ok", "db": "ok", "uptime_seconds": N }`.
- **Logging:** `RUST_LOG=info,ferrisctf=debug`. No flag values, flag hashes of correct answers, or password hashes are logged.
- **Graceful shutdown:** drain in-flight requests within 10 seconds before exit.

## 15. Event Start Checklist

1. Ferris has uploaded challenges (with difficulty set), hints, and attachments, and the bank roughly matches PLANNED\_QUESTIONS.
2. `SERVER_SECRET` and admin credentials are set in the environment and backed up.
3. Ferris triggers `/admin/event/start` at the scheduled time, which starts Round 1 (optionally with a time limit).
4. The system locks all valid teams and assigns each its pool for Round 1.
5. Free agents have 30 minutes to form or join a valid team.
6. Scoreboard polling begins and hints are available on challenge pages.
7. Ferris ends each round (or lets the time limit end it) and starts the next one until the planned rounds are done.

## Revision Notes

Corrections made relative to the draft:

- **One team per user:** `team_members` PK did not enforce it. Added a unique index on `user_id`.
- **Grace-period contradiction:** free agents could not join once teams were locked. Clarified that unlocked or new teams may form during the grace window and lock when valid.
- **Event state:** added `event_state` table (start timestamp and scheduled start), which the draft referenced but never defined.
- **Sessions:** cookie lacked a session ID, so revocation was impossible. Added `session_id`, DB re-check for ban and admin status.
- **Logout:** changed GET to POST so it is CSRF-protected.
- **Admin login:** documented the unavoidable exception to "admin looks like 404". Replaced the separate constant-time credential compare with Argon2 verification and a dummy hash for unknown users.
- **Dependencies:** added `sqlx` `macros` feature, `hex`, `thiserror`, `num_cpus`, and `tower-http` `limit`.
- **Flag verifier:** replaced undefined `zeroize(trimmed_buf)` with a working `Zeroizing` implementation.
- **Download handler:** fixed the mixed-type header array that would not compile, bound UUIDs as strings for SQLite, required active challenge and locked team, and added Content-Length.
- **Pool and submissions:** specified that each pool keeps 5 unsolved challenges, and added submission preconditions (pool membership, not already solved, active, not banned).
- **Schema hardening:** `audit_log` admin FK changed to RESTRICT, millisecond `solved_at` for tie-breaking, `CHECK` constraints on points and costs, composite submissions index.
- **Scoreboard:** excludes unlocked teams.
- **Routes:** added missing routes for static assets, challenge delete, hint edit/delete, bulk toggle, user ban, and CSV export. Removed the undefined "promote" action and marked join-request approval as out of scope.
- **Rate limits:** noted that per-team limits need a custom key extractor, and that `/team/preview/:code` shares the join limit to prevent code enumeration.
- **Operations:** DB pool reduced from 50 to 16 with `BEGIN IMMEDIATE`, raised Axum body limit for uploads, embedded templates for true single-binary delivery, and a warning that changing `SERVER_SECRET` invalidates all flags.

Changes made in v3 (relative to v2):

- **Team size:** teams are now 1 to 2 members (TEAM\_MIN\_SIZE 1, TEAM\_MAX\_SIZE 2). A single participant is a team. One captain per team, enforced by a partial unique index. The captain chooses the team name shown on the leaderboard, and member display names are shown too. Undersized-team disbanding at event start no longer applies.
- **Rounds:** added the `rounds` table and Section 6.4. Ferris alone starts rounds, ends rounds, and sets or changes the time limit that ends a round automatically. Submissions are accepted only during an active round.
- **Pools:** replaced the fixed 5-unsolved pool with a per-round pool (default 15 from 60 planned questions over 4 rounds), split 60/30/10 easy/medium/hard by largest remainder. Pools are redistributed from the question bank each round, independently per team, excluding solved challenges. Mid-round replenishment on solve was removed, and `team_challenge_pool` now carries `round_number`.
- **Repeat penalty:** unsolved challenges that reappear lose 10 points per re-appearance, never below zero (frozen at assignment, combined with hint costs under the same floor).
- **Difficulty:** added `challenges.difficulty` (easy, medium, hard), assigned at creation and not editable.
- **Tie-breaking:** replaced the score, last-solve-time, creation-time order with score, then countback (hard, medium, easy counts), then countback by cumulative solve counts (lower wins), then time first reaching the final score, then team creation time. Added `solves.round_number`.
- **Roles:** Ferris now has a web identity (`is_ferris`, set on the bootstrap account), adds and removes admins, controls rounds, and manages challenges and hints. Admins are limited to participants and scores.
- **Score adjustments:** admins and Ferris may adjust scores manually through the new `score_adjustments` table, with a required reason and an `audit_log` entry.
- **Routes:** added round, admin-management, and score-adjustment routes, and moved event start and challenge and hint management to Ferris.

Changes made in v5 (relative to v3):

- **Upload limit:** MAX\_UPLOAD\_BYTES raised from 10 MB to 30 MB (31457280). Applies to the constants table, the upload rules in 9.6, and the file service in Section 13.
- **Clearer instructions:** hint definitions are now consistently attributed to Ferris (6.3 previously said Admin); the 404 principle in Section 1 now covers Ferris-only routes; the ranking order in 7.1 states that tie-break steps apply only between teams with equal total score; 6.4 states that ROUND\_COUNT sizes pools but does not cap rounds; "out of scope for v2" now reads "for this version"; title and revision pointers updated.
