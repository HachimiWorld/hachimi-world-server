-- Package metadata for in-app updates: the client downloads `url` in the background,
-- resumes it with `size` and verifies it against `sha256` before installing.
-- Nullable because existing rows were published without them.
ALTER TABLE version ADD COLUMN size   BIGINT   DEFAULT NULL;
ALTER TABLE version ADD COLUMN sha256 CHAR(64) DEFAULT NULL;
