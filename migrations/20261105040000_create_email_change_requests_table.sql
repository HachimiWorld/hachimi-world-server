-- Changes of a user's email.
-- method 'old_email': the user proved they can read both the old and the new address, applied at once.
-- method 'wait': the user can't read the old address, so they gave their password instead. The change
-- applies at effective_time; until then, and for a while after, the old address can undo it with the
-- link it was sent (revert_token_hash).
-- status: pending (waiting for effective_time), applied, cancelled (before applying), reverted (after
-- applying), failed (the new address was taken by then).
CREATE TABLE email_change_requests
(
    id                  BIGSERIAL PRIMARY KEY,
    user_id             BIGINT      NOT NULL,
    old_email           TEXT        NOT NULL,
    new_email           TEXT        NOT NULL,
    method              TEXT        NOT NULL,
    status              TEXT        NOT NULL,
    effective_time      TIMESTAMPTZ NOT NULL,
    -- SHA-256 of the undo token, hex. The token itself is only in the emails. Null for 'old_email'.
    revert_token_hash   TEXT UNIQUE,
    -- The undo link works until this time.
    revert_expire_time  TIMESTAMPTZ,
    -- Reminder emails queued for the old address, deleted if the request ends before they are sent.
    reminder_email_ids  UUID[]      NOT NULL DEFAULT '{}',
    ip_address          TEXT,
    user_agent          TEXT,
    finish_time         TIMESTAMPTZ, -- When it left 'pending', or was reverted
    create_time         TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_email_change_requests_user_id ON email_change_requests (user_id, create_time);
CREATE INDEX idx_email_change_requests_pending ON email_change_requests (effective_time) WHERE status = 'pending';
