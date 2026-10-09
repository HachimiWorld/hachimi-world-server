-- One case per reported target, reused forever: a resolved case reopens when reported again.
CREATE TABLE report_cases
(
    id               BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    target_type      TEXT        NOT NULL, -- song / playlist / user
    target_id        BIGINT      NOT NULL,
    target_owner_uid BIGINT      NOT NULL,
    status           TEXT        NOT NULL, -- pending / resolved
    ignore_reports   BOOLEAN     NOT NULL, -- new reports are recorded but don't reopen the case
    pending_count    INT         NOT NULL,
    last_report_time TIMESTAMPTZ NOT NULL,
    create_time      TIMESTAMPTZ NOT NULL,
    update_time      TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_report_cases_target ON report_cases (target_type, target_id);
CREATE INDEX idx_report_cases_queue ON report_cases (status, last_report_time DESC, id DESC);

CREATE TABLE reports
(
    id           BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    case_id      BIGINT      NOT NULL,
    reporter_uid BIGINT      NOT NULL,
    reason       TEXT        NOT NULL,
    detail       TEXT,
    action_id    BIGINT, -- the moderation action that handled it; NULL while pending
    create_time  TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_reports_case ON reports (case_id, id);
CREATE INDEX idx_reports_reporter_time ON reports (reporter_uid, create_time DESC);

-- Append-only: one row per decision on a case, also the audit log.
CREATE TABLE moderation_actions
(
    id              BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    case_id         BIGINT      NOT NULL,
    operator_uid    BIGINT      NOT NULL,
    verdict         TEXT        NOT NULL, -- agree / disagree / ignore
    note            TEXT,
    ignore_reports  BOOLEAN     NOT NULL,
    up_to_report_id BIGINT      NOT NULL, -- reports up to this id were handled
    target_snapshot JSONB       NOT NULL, -- the target as the operator saw it
    content_actions TEXT[]      NOT NULL, -- what it did to the content, such as hide or reset_bio
    author_reason   TEXT,                 -- why, as told to the content's owner
    create_time     TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_moderation_actions_case ON moderation_actions (case_id, id DESC);

CREATE TABLE committee_members
(
    uid              BIGINT PRIMARY KEY,
    appointed_by_uid BIGINT      NOT NULL,
    create_time      TIMESTAMPTZ NOT NULL
);

-- Hidden by the platform: only the owner can see it, and the owner can't change it. The default
-- covers existing rows and instances still running the previous version during a deploy.
ALTER TABLE songs ADD COLUMN is_hidden BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE playlists ADD COLUMN is_hidden BOOLEAN NOT NULL DEFAULT FALSE;
