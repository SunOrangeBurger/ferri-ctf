CREATE TABLE honeypot_pending (
    team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    challenge_id TEXT NOT NULL REFERENCES challenges(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id),
    submitted_at TEXT NOT NULL,
    due_at TEXT NOT NULL,
    applied_at TEXT,
    PRIMARY KEY(team_id,challenge_id)
);
CREATE INDEX honeypot_due ON honeypot_pending(due_at) WHERE applied_at IS NULL;
