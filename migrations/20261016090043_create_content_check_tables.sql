-- One row per version of the content check policy, so every check can be traced to the exact
-- questions and thresholds it was judged by. The policy itself lives in deployment config.
CREATE TABLE content_policies
(
    version     TEXT PRIMARY KEY,     -- first 12 hex digits of the sha256 of the policy directory; not ordered, sort by create_time
    files       JSONB       NOT NULL, -- relative path -> file content
    create_time TIMESTAMPTZ NOT NULL
);

-- One row per piece of content checked (a text or an image), passed or blocked.
CREATE TABLE content_checks
(
    id             BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    subject        TEXT        NOT NULL, -- what was checked, such as user.username or user.bio
    target_id      BIGINT      NOT NULL, -- the object the content belongs to
    actor_uid      BIGINT      NOT NULL, -- who submitted it
    content        TEXT        NOT NULL, -- the text as submitted, or the storage key of an image; a blocked text is kept only here
    content_hash   TEXT        NOT NULL, -- sha256 hex of exactly what was checked: the text as submitted, or the image file
    policy_version TEXT        NOT NULL,
    model          TEXT,                 -- the model that answered; NULL when the call failed
    answers        JSONB,                -- the answers it returned; NULL when the call failed
    error          TEXT,                 -- why the call failed; NULL when it succeeded
    blocked_by     TEXT[]      NOT NULL, -- questions that reached their threshold; empty when passed
    latency_ms     INT         NOT NULL,
    create_time    TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_content_checks_actor_time ON content_checks (actor_uid, create_time DESC);
