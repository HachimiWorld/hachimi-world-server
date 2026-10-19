-- Whether the owner uploaded the cover, set by `/playlist/set_cover`. Covers uploaded before this
-- are marked by a later migration, once every server sets it.
ALTER TABLE playlists ADD COLUMN custom_cover BOOLEAN NOT NULL DEFAULT FALSE;
