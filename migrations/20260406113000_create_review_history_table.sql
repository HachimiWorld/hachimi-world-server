CREATE TABLE song_publishing_review_history
(
    id            BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY NOT NULL,
    review_id     BIGINT                                          NOT NULL,
    user_id       BIGINT                                          NOT NULL,
    action_type   INT                                             NOT NULL,
    note          TEXT,
    snapshot_data JSONB                                           NOT NULL,
    create_time   TIMESTAMPTZ                                     NOT NULL
);

CREATE INDEX idx_song_publishing_review_history_review_id_create_time
    ON song_publishing_review_history (review_id, create_time DESC, id DESC);

