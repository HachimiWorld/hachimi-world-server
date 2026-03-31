CREATE TABLE user_play_history
(
    user_id     BIGINT                   NOT NULL,
    song_id     BIGINT                   NOT NULL,
    create_time TIMESTAMP WITH TIME ZONE NOT NULL,
    PRIMARY KEY (user_id, song_id)
);
COMMENT ON TABLE user_play_history IS 'This is the table that records the play history of users, but only keeps the most recent play record for each song.';

CREATE INDEX idx_user_play_history_user_id_create_time ON user_play_history (user_id, create_time DESC);
COMMENT ON INDEX idx_user_play_history_user_id_create_time IS 'Used to quickly retrieve the most recent play history of a user, ordered by create_time in descending order.';