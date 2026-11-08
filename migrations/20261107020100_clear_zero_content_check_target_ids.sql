-- 261106 and the release after it wrote 0 for a target_id not known yet; it's null from now on.
-- Instances of the release before may still write a few 0s while this deploys; they're read as null
-- anyway.
UPDATE content_checks SET target_id = NULL WHERE target_id = 0;
UPDATE content_reviews SET target_id = NULL WHERE target_id = 0;
