-- Existing download-only challenges keep their current behavior.
ALTER TABLE challenges ADD COLUMN challenge_url TEXT;
