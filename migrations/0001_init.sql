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
    scheduled_start DATETIME,
    started_at DATETIME                     -- NULL until event starts (Round 1 start)
);
INSERT OR IGNORE INTO event_state (id) VALUES (1);

-- Rounds (waves). Only Ferris creates, ends, and sets limits on rounds.
CREATE TABLE IF NOT EXISTS rounds (
    round_number INTEGER PRIMARY KEY,
    started_at DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f', 'now')),
    time_limit_minutes INTEGER CHECK (time_limit_minutes IS NULL OR time_limit_minutes > 0),
    auto_end_at DATETIME,                   -- started_at + time limit; NULL = no limit
    ended_at DATETIME                       -- NULL while the round is active
);

-- Teams (rooms)
CREATE TABLE IF NOT EXISTS teams (
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    join_code TEXT UNIQUE NOT NULL,
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
CREATE UNIQUE INDEX IF NOT EXISTS idx_team_members_one_team ON team_members(user_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_team_members_one_captain ON team_members(team_id) WHERE role = 'captain';

-- Challenges
CREATE TABLE IF NOT EXISTS challenges (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    category TEXT NOT NULL,                 -- Web | Pwn | Crypto | Forensics | Rev | Misc
    difficulty TEXT NOT NULL CHECK (difficulty IN ('easy', 'medium', 'hard')),
    points INTEGER NOT NULL CHECK (points >= 0),
    flag_hmac TEXT NOT NULL,
    flag_salt TEXT NOT NULL,
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

-- Per-team, per-round challenge pool
CREATE TABLE IF NOT EXISTS team_challenge_pool (
    team_id TEXT NOT NULL,
    round_number INTEGER NOT NULL,
    challenge_id TEXT NOT NULL,
    repeat_penalty INTEGER NOT NULL DEFAULT 0 CHECK (repeat_penalty >= 0),
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
    submitted_hash TEXT NOT NULL,
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
    round_number INTEGER NOT NULL,
    points_awarded INTEGER NOT NULL,
    solved_at DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f', 'now')),
    PRIMARY KEY (team_id, challenge_id),
    FOREIGN KEY(team_id) REFERENCES teams(id) ON DELETE CASCADE,
    FOREIGN KEY(challenge_id) REFERENCES challenges(id) ON DELETE CASCADE,
    FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY(round_number) REFERENCES rounds(round_number) ON DELETE CASCADE
);

-- Manual score adjustments by admins or Ferris
CREATE TABLE IF NOT EXISTS score_adjustments (
    id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL,
    admin_id TEXT NOT NULL,
    delta INTEGER NOT NULL,
    reason TEXT NOT NULL,
    created_at DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f', 'now')),
    FOREIGN KEY(team_id) REFERENCES teams(id) ON DELETE CASCADE,
    FOREIGN KEY(admin_id) REFERENCES users(id) ON DELETE RESTRICT
);

-- Admin audit log
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
