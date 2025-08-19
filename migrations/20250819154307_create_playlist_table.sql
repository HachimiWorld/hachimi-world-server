CREATE TABLE playlists
(
    id          BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    name        VARCHAR(255)             NOT NULL,
    description TEXT,
    user_id     BIGINT                   NOT NULL,
    cover_url   TEXT,
    is_public   BOOLEAN                  NOT NULL DEFAULT FALSE,
    create_time TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    update_time TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_playlists_user_id ON playlists (user_id);
CREATE INDEX idx_playlists_is_public ON playlists (is_public);

CREATE TABLE playlist_songs
(
    playlist_id BIGINT                   NOT NULL,
    song_id     BIGINT                   NOT NULL,
    order_index INT                      NOT NULL,
    add_time    TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    PRIMARY KEY (playlist_id, song_id)
);
CREATE INDEX idx_playlist_songs_playlist_id ON playlist_songs (playlist_id);
CREATE INDEX idx_playlist_songs_song_id ON playlist_songs (song_id);