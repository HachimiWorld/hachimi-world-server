-- This is used to store creator information, especially for jmid
CREATE TABLE creators
(
    id          BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    user_id     BIGINT      NOT NULL,
    jmid_prefix VARCHAR(8)  NOT NULL,
    active      BOOLEAN     NOT NULL,
    create_time TIMESTAMPTZ NOT NULL,
    update_time TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_creators_jmid_prefix ON creators (jmid_prefix);
CREATE INDEX idx_creators_user_id ON creators (user_id);

ALTER TABLE song_publishing_review
    ADD type INT NOT NULL DEFAULT 0; -- 0: create, 1: modify

ALTER TABLE song_publishing_review
    ADD comment TEXT DEFAULT NULL;
-- 0: create, 1: modify

-- Create jmid index for publishing_review table because we need to list PRs by jmid
CREATE INDEX idx_song_publishing_review_display_id ON song_publishing_review (song_display_id)