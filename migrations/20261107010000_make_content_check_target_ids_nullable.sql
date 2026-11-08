-- target_id is null for content checked before it's saved, until it's saved, and for good if it never
-- is. 261106 wrote 0 for that, which is read as null. Writing null waits for the next release: until
-- every instance runs this one, an older one may read these rows and can't take a null.
ALTER TABLE content_checks ALTER COLUMN target_id DROP NOT NULL;
ALTER TABLE content_reviews ALTER COLUMN target_id DROP NOT NULL;
