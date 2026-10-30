-- Comments on any kind of content, told apart by target_type. Only what every kind of comment has
-- goes here; something only one kind needs goes in a table of that kind, keyed by comment id.
-- Two levels: a comment on the target, and replies under it.
CREATE TABLE comments
(
    id                  BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    target_type         TEXT        NOT NULL, -- song
    target_id           BIGINT      NOT NULL,
    user_id             BIGINT      NOT NULL,
    root_id             BIGINT,               -- the top-level comment a reply is under; NULL for a top-level comment
    reply_to_id         BIGINT,               -- the comment replied to; NULL for a top-level comment
    reply_to_uid        BIGINT      NOT NULL, -- who is replied to: the author of reply_to_id, or the target's owner
    content             TEXT        NOT NULL,
    status              TEXT        NOT NULL, -- pending / published / rejected, set by content checks
    is_hidden           BOOLEAN     NOT NULL, -- hidden by a report decision
    like_count          INT         NOT NULL,
    visible_reply_count INT         NOT NULL,
    delete_time         TIMESTAMPTZ,          -- deleted by its author or the target's owner
    create_time         TIMESTAMPTZ NOT NULL,
    update_time         TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_comments_target_root ON comments (target_type, target_id, root_id, id DESC);
CREATE INDEX idx_comments_root ON comments (root_id, id);
CREATE INDEX idx_comments_reply_to_uid ON comments (reply_to_uid, create_time DESC);
CREATE INDEX idx_comments_user_time ON comments (user_id, create_time DESC);

CREATE TABLE comment_likes
(
    comment_id  BIGINT      NOT NULL,
    user_id     BIGINT      NOT NULL,
    create_time TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (comment_id, user_id)
);

-- Content waiting for a content check that couldn't be done when it was submitted, retried in the
-- background. The content itself stays in its own table.
CREATE TABLE content_reviews
(
    id             BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    subject        TEXT        NOT NULL, -- the content check subject, such as song.comment
    target_id      BIGINT      NOT NULL, -- the content, such as the comment id
    attempt_count  INT         NOT NULL,
    available_time TIMESTAMPTZ NOT NULL, -- not tried before this
    done_time      TIMESTAMPTZ,          -- when it was settled
    last_error     TEXT,
    create_time    TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_content_reviews_due ON content_reviews (available_time) WHERE done_time IS NULL;
