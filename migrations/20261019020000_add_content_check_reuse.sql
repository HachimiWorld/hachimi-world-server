-- A check whose content was already judged under the same policy and model reuses that answer
-- instead of asking TypeSafe again.
ALTER TABLE content_checks ADD COLUMN reused_from BIGINT; -- the check whose answer this one reused; NULL when TypeSafe was asked

-- Finds a successful earlier check of the same content
CREATE INDEX idx_content_checks_hash ON content_checks (content_hash, subject, policy_version) WHERE error IS NULL;
