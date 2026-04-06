CREATE TABLE song_publishing_review_comment
(
    id          BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY NOT NULL,
    review_id   BIGINT                                          NOT NULL,
    user_id     BIGINT                                          NOT NULL,
    content     TEXT                                            NOT NULL,
    create_time TIMESTAMPTZ                                     NOT NULL,
    update_time TIMESTAMPTZ                                     NOT NULL
);

CREATE INDEX idx_song_publishing_review_comment_review_id_create_time ON song_publishing_review_comment (
    review_id, create_time
);