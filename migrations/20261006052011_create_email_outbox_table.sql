-- Emails waiting to be sent by the relay. Rows are written in the transaction of the change they
-- describe, so an email goes out if and only if that change is committed.
-- Pending: sent_time and dead_time are both null. Due: pending and available_time has passed.
CREATE TABLE email_outbox
(
    id             UUID PRIMARY KEY NOT NULL, -- UUIDv7
    to_address     TEXT             NOT NULL,
    subject        TEXT             NOT NULL,
    body           TEXT             NOT NULL, -- Plain text
    available_time TIMESTAMPTZ      NOT NULL, -- Not sent before this; pushed back after a failure
    attempt_count  INT              NOT NULL,
    sent_time      TIMESTAMPTZ,
    dead_time      TIMESTAMPTZ,               -- Given up after too many failures
    last_error     TEXT,
    create_time    TIMESTAMPTZ      NOT NULL
);

CREATE INDEX idx_email_outbox_pending ON email_outbox (available_time) WHERE sent_time IS NULL AND dead_time IS NULL;
CREATE INDEX idx_email_outbox_sent_time ON email_outbox (sent_time) WHERE sent_time IS NOT NULL;
CREATE INDEX idx_email_outbox_dead_time ON email_outbox (dead_time) WHERE dead_time IS NOT NULL;
