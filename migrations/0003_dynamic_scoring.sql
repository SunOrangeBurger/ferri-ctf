-- Opt-in scoring; existing contests retain static points and historical awards.
CREATE TABLE scoring_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    revision INTEGER NOT NULL DEFAULT 0
);
INSERT INTO scoring_settings (id) VALUES (1);

CREATE TABLE difficulty_scoring_rules (
    difficulty TEXT PRIMARY KEY CHECK (difficulty IN ('easy', 'medium', 'hard')),
    starting_points INTEGER NOT NULL,
    minimum_points INTEGER NOT NULL CHECK (minimum_points >= 0),
    maximum_points INTEGER NOT NULL CHECK (maximum_points <= 1000000),
    quiet_minutes INTEGER NOT NULL CHECK (quiet_minutes BETWEEN 1 AND 10080),
    recovery_per_minute INTEGER NOT NULL CHECK (recovery_per_minute BETWEEN 1 AND 1000000),
    drop_percent INTEGER NOT NULL CHECK (drop_percent BETWEEN 1 AND 99),
    CHECK (minimum_points <= starting_points AND starting_points <= maximum_points)
);
INSERT INTO difficulty_scoring_rules VALUES
    ('easy', 100, 40, 150, 15, 1, 15),
    ('medium', 300, 175, 400, 20, 2, 10),
    ('hard', 700, 450, 1000, 30, 3, 5);

-- Values use millionths of a point; clocks use cumulative available milliseconds.
CREATE TABLE challenge_pricing (
    challenge_id TEXT PRIMARY KEY REFERENCES challenges(id) ON DELETE CASCADE,
    anchor_units INTEGER NOT NULL CHECK (anchor_units >= 0),
    anchor_active_ms INTEGER NOT NULL DEFAULT 0,
    last_solve_active_ms INTEGER NOT NULL DEFAULT 0,
    ceiling_units INTEGER
);
CREATE TABLE challenge_availability (
    id INTEGER PRIMARY KEY,
    challenge_id TEXT NOT NULL REFERENCES challenges(id) ON DELETE CASCADE,
    round_number INTEGER NOT NULL REFERENCES rounds(round_number) ON DELETE CASCADE,
    opened_at TEXT NOT NULL,
    closed_at TEXT
);
CREATE UNIQUE INDEX idx_availability_open
    ON challenge_availability(challenge_id, round_number) WHERE closed_at IS NULL;
CREATE INDEX idx_availability_challenge ON challenge_availability(challenge_id);

-- Triggers keep every existing pool/activation path consistent, including bulk
-- updates. Round ends/deadlines bound these windows when the clock is read.
CREATE TRIGGER scoring_pool_insert AFTER INSERT ON team_challenge_pool
WHEN (SELECT enabled FROM scoring_settings WHERE id = 1) = 1
BEGIN
    INSERT OR IGNORE INTO challenge_availability(challenge_id, round_number, opened_at)
    SELECT NEW.challenge_id, NEW.round_number, strftime('%Y-%m-%d %H:%M:%f', 'now')
    FROM challenges c JOIN rounds r ON r.round_number = NEW.round_number
    JOIN teams t ON t.id = NEW.team_id
    WHERE c.id = NEW.challenge_id AND c.is_active = 1 AND t.is_locked = 1 AND t.disbanded = 0
      AND r.ended_at IS NULL AND (r.auto_end_at IS NULL OR r.auto_end_at > strftime('%Y-%m-%d %H:%M:%f', 'now'));
    INSERT OR IGNORE INTO challenge_pricing(challenge_id, anchor_units)
    SELECT c.id, d.starting_points * 1000000 FROM challenges c
    JOIN difficulty_scoring_rules d ON d.difficulty = c.difficulty
    WHERE c.id = NEW.challenge_id AND EXISTS (
        SELECT 1 FROM challenge_availability a WHERE a.challenge_id = c.id
    );
END;

CREATE TRIGGER scoring_challenge_visibility AFTER UPDATE OF is_active ON challenges
WHEN OLD.is_active != NEW.is_active AND (SELECT enabled FROM scoring_settings WHERE id = 1) = 1
BEGIN
    UPDATE challenge_availability SET closed_at = strftime('%Y-%m-%d %H:%M:%f', 'now')
    WHERE challenge_id = NEW.id AND closed_at IS NULL AND NEW.is_active = 0;
    INSERT OR IGNORE INTO challenge_availability(challenge_id, round_number, opened_at)
    SELECT NEW.id, r.round_number, strftime('%Y-%m-%d %H:%M:%f', 'now')
    FROM rounds r WHERE NEW.is_active = 1 AND r.ended_at IS NULL
      AND (r.auto_end_at IS NULL OR r.auto_end_at > strftime('%Y-%m-%d %H:%M:%f', 'now'))
      AND EXISTS (SELECT 1 FROM team_challenge_pool p JOIN teams t ON t.id = p.team_id
          WHERE p.challenge_id = NEW.id AND p.round_number = r.round_number
            AND t.is_locked = 1 AND t.disbanded = 0);
    INSERT OR IGNORE INTO challenge_pricing(challenge_id, anchor_units)
    SELECT NEW.id, starting_points * 1000000 FROM difficulty_scoring_rules
    WHERE difficulty = NEW.difficulty AND EXISTS (
        SELECT 1 FROM challenge_availability WHERE challenge_id = NEW.id
    );
END;

CREATE TRIGGER scoring_team_visibility AFTER UPDATE OF is_locked, disbanded ON teams
WHEN (SELECT enabled FROM scoring_settings WHERE id = 1) = 1
BEGIN
    UPDATE challenge_availability SET closed_at = strftime('%Y-%m-%d %H:%M:%f', 'now')
    WHERE closed_at IS NULL AND NOT EXISTS (
        SELECT 1 FROM team_challenge_pool p JOIN teams t ON t.id = p.team_id
        WHERE p.challenge_id = challenge_availability.challenge_id
          AND p.round_number = challenge_availability.round_number
          AND t.is_locked = 1 AND t.disbanded = 0
    );
    INSERT OR IGNORE INTO challenge_availability(challenge_id, round_number, opened_at)
    SELECT p.challenge_id, p.round_number, strftime('%Y-%m-%d %H:%M:%f', 'now')
    FROM team_challenge_pool p JOIN challenges c ON c.id = p.challenge_id
    JOIN rounds r ON r.round_number = p.round_number
    WHERE p.team_id = NEW.id AND NEW.is_locked = 1 AND NEW.disbanded = 0 AND c.is_active = 1
      AND r.ended_at IS NULL AND (r.auto_end_at IS NULL OR r.auto_end_at > strftime('%Y-%m-%d %H:%M:%f', 'now'));
    INSERT OR IGNORE INTO challenge_pricing(challenge_id, anchor_units)
    SELECT c.id, d.starting_points * 1000000 FROM challenges c
    JOIN difficulty_scoring_rules d ON d.difficulty = c.difficulty
    WHERE EXISTS (SELECT 1 FROM challenge_availability a WHERE a.challenge_id = c.id);
END;

CREATE TRIGGER scoring_pool_delete AFTER DELETE ON team_challenge_pool
WHEN (SELECT enabled FROM scoring_settings WHERE id = 1) = 1
BEGIN
    UPDATE challenge_availability SET closed_at = strftime('%Y-%m-%d %H:%M:%f', 'now')
    WHERE challenge_id = OLD.challenge_id AND round_number = OLD.round_number AND closed_at IS NULL
      AND NOT EXISTS (SELECT 1 FROM team_challenge_pool p JOIN teams t ON t.id = p.team_id
          WHERE p.challenge_id = OLD.challenge_id AND p.round_number = OLD.round_number
            AND t.is_locked = 1 AND t.disbanded = 0);
END;
