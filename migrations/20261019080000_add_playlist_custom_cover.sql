-- Whether the owner uploaded the cover. When FALSE, cover_url is the cover of the most recently
-- added song that isn't hidden, refreshed when songs are added or removed.
ALTER TABLE playlists ADD COLUMN custom_cover BOOLEAN NOT NULL DEFAULT FALSE;

-- Every cover so far was uploaded by its owner
UPDATE playlists SET custom_cover = TRUE WHERE cover_url IS NOT NULL;
