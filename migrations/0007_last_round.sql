ALTER TABLE rounds ADD COLUMN is_last_round INTEGER NOT NULL DEFAULT 0 CHECK (is_last_round IN (0, 1));
CREATE UNIQUE INDEX one_last_round ON rounds(is_last_round) WHERE is_last_round = 1;
-- Freeze eligibility when LAST ROUND starts; later solves/uploads cannot expand it.
CREATE TABLE last_round_challenges (
    round_number INTEGER NOT NULL REFERENCES rounds(round_number),
    challenge_id TEXT NOT NULL REFERENCES challenges(id),
    PRIMARY KEY (round_number, challenge_id)
);
