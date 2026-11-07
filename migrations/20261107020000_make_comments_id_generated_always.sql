-- Comments get their id from the database since 261106; until then the server took ids from the
-- sequence itself, before the content check, and inserted them. Apply only once every instance runs
-- 261106 or later: an older one still inserts its own ids, which this refuses.
ALTER TABLE comments ALTER COLUMN id SET GENERATED ALWAYS;
