CREATE TABLE system_notices (
    id TEXT PRIMARY KEY,
    message TEXT NOT NULL,
    created_at DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f', 'now')),
    expires_at DATETIME
);
CREATE INDEX idx_system_notices_visible ON system_notices(created_at, expires_at);
