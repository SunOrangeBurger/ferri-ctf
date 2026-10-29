-- Special rounds contain five selective challenges shared by every team.
ALTER TABLE rounds ADD COLUMN is_selective_only INTEGER NOT NULL DEFAULT 0;
