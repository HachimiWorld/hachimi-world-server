CREATE TABLE posts
(
    id           BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    author_uid   BIGINT      NOT NULL,
    title        TEXT        NOT NULL,
    content      TEXT        NOT NULL,
    content_type TEXT        NOT NULL,
    cover_url    TEXT,
    create_time  TIMESTAMPTZ NOT NULL,
    update_time  TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_posts_author_uid_create_time ON posts (author_uid, create_time DESC);
CREATE INDEX idx_posts_create_time ON posts (create_time DESC);