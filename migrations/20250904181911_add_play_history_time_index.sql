-- for play history
ALTER TABLE song_plays
    ALTER COLUMN create_time SET NOT NULL;
CREATE INDEX idx_song_plays_user_time ON song_plays (user_id, create_time DESC);