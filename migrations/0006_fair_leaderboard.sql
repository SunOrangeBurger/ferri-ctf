-- Freeze the value offered to a team so later challenge/rule edits cannot
-- retroactively change its pool-adjusted leaderboard position.
ALTER TABLE team_challenge_pool
ADD COLUMN opportunity_points INTEGER NOT NULL DEFAULT 0 CHECK (opportunity_points >= 0);

UPDATE team_challenge_pool
SET opportunity_points = COALESCE((
    SELECT CASE WHEN settings.enabled = 1
                THEN rules.maximum_points ELSE c.points END
    FROM challenges c
    CROSS JOIN scoring_settings settings
    LEFT JOIN difficulty_scoring_rules rules ON rules.difficulty = c.difficulty
    WHERE c.id = team_challenge_pool.challenge_id
), 0);
