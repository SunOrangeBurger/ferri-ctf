-- Server-side sessions. The raw token exists only in the user's cookie;
-- the DB stores its SHA-256, so a leaked DB file cannot be replayed as logins.
CREATE TABLE IF NOT EXISTS sessions (
    id_hash TEXT PRIMARY KEY,               -- hex(SHA-256(token))
    user_id TEXT NOT NULL,
    created_at DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f', 'now')),
    expires_at DATETIME NOT NULL,           -- absolute expiry, same text format as created_at
    FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_expiry ON sessions(expires_at);
