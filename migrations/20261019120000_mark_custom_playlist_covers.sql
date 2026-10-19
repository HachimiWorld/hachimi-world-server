-- Every server now sets custom_cover when a cover is uploaded, and none has set a default cover
-- yet, so every cover so far was uploaded by its owner
UPDATE playlists SET custom_cover = TRUE WHERE cover_url IS NOT NULL;
