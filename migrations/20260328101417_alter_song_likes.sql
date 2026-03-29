DROP TABLE IF EXISTS song_likes;

CREATE TABLE song_likes
(
    song_id                BIGINT                   NOT NULL,
    user_id                BIGINT                   NOT NULL,
    playback_position_secs INT,
    create_time            TIMESTAMP WITH TIME ZONE NOT NULL,
    PRIMARY KEY (song_id, user_id)
);
CREATE INDEX idx_song_likes_song_id ON song_likes (song_id);
CREATE INDEX idx_song_likes_user_id ON song_likes (user_id);
