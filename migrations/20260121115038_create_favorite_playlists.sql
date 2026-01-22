CREATE TABLE favorite_playlists
(
    user_id     BIGINT      NOT NULL,
    playlist_id BIGINT      NOT NULL,
    order_index INT         NOT NULL,
    add_time    TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (user_id, playlist_id)
);

CREATE INDEX idx_favorite_playlists_user_id ON favorite_playlists(user_id);
CREATE INDEX idx_favorite_playlists_playlist_id ON favorite_playlists(playlist_id);