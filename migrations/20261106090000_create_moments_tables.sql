-- Moments: short posts any user can write, with up to 9 images and attachments such as a song or a
-- playlist. Only moments that passed their content checks, or were published without them, are
-- here; one waiting for a check is in content_reviews.
CREATE TABLE moments
(
    id          BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    user_id     BIGINT      NOT NULL,
    content     TEXT        NOT NULL, -- may be empty when there are images or attachments
    images      JSONB       NOT NULL, -- [{ url, thumb_url, width, height }], in order
    attachments JSONB       NOT NULL, -- [{ type, data }], in order; each type decides the shape of its data
    is_hidden   BOOLEAN     NOT NULL, -- hidden by a report decision
    like_count  INT         NOT NULL,
    delete_time TIMESTAMPTZ,          -- deleted by its author
    create_time TIMESTAMPTZ NOT NULL,
    update_time TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_moments_user_time ON moments (user_id, create_time DESC, id DESC);
CREATE INDEX idx_moments_time ON moments (create_time DESC, id DESC);

CREATE TABLE moment_likes
(
    moment_id   BIGINT      NOT NULL,
    user_id     BIGINT      NOT NULL,
    create_time TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (moment_id, user_id)
);
