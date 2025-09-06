CREATE TABLE song_publishing_review
(
    id              BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    user_id         BIGINT      NOT NULL,
    song_display_id VARCHAR(16) NOT NULL,
    data            JSONB       NOT NULL,
    submit_time     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    update_time     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    review_time     TIMESTAMPTZ,
    review_comment  TEXT,
    status          INT         NOT NULL -- 0=Pending, 1=Approved, 2=Rejected
);

CREATE INDEX idx_song_audio_user_id_status ON song_publishing_review (user_id, status);
CREATE INDEX idx_song_audio_status ON song_publishing_review (status);