-- When each user last read a kind of message that is read as a whole rather than one by one, such
-- as received likes and new followers. Messages after read_time are unread.
CREATE TABLE message_read_marks
(
    uid         BIGINT      NOT NULL,
    channel     TEXT        NOT NULL, -- 'like' | 'follow'
    read_time   TIMESTAMPTZ NOT NULL,
    update_time TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (uid, channel)
);

-- Received likes are listed and counted by song and time
CREATE INDEX idx_song_likes_song_id_create_time ON song_likes (song_id, create_time DESC);
