ALTER TABLE rounds ADD COLUMN is_backlog_round INTEGER NOT NULL DEFAULT 0 CHECK (is_backlog_round IN (0,1));
CREATE TABLE backlog_round_challenges (
    round_number INTEGER NOT NULL REFERENCES rounds(round_number),
    challenge_id TEXT NOT NULL REFERENCES challenges(id),
    PRIMARY KEY (round_number,challenge_id)
);
