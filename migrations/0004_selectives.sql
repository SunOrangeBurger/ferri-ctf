-- Selectives: a special challenge category chosen by admins per round.
-- Each round, up to 5 selectives are picked; each team randomly gets 2 of the 5.
-- Once used in a round, a selective is locked out from future rounds unless explicitly overridden.

-- Mark challenges as selective (set at upload time)
ALTER TABLE challenges ADD COLUMN is_selective INTEGER NOT NULL DEFAULT 0;

-- Track which selectives are chosen for each round (up to 5 per round)
CREATE TABLE IF NOT EXISTS round_selectives (
    round_number INTEGER NOT NULL,
    challenge_id TEXT NOT NULL,
    PRIMARY KEY (round_number, challenge_id),
    FOREIGN KEY(round_number) REFERENCES rounds(round_number) ON DELETE CASCADE,
    FOREIGN KEY(challenge_id) REFERENCES challenges(id) ON DELETE CASCADE
);

-- Index for quick "has this selective been used?" lookups
CREATE INDEX IF NOT EXISTS idx_round_selectives_challenge
    ON round_selectives(challenge_id);
