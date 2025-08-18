-- Songs table - main song information
CREATE TABLE songs
(
    id               BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    display_id       VARCHAR(16)              NOT NULL UNIQUE,
    title            VARCHAR(255)             NOT NULL,
    artist           VARCHAR(255)             NOT NULL,
    file_url         VARCHAR(500)             NOT NULL, -- S3/R2 URL to audio file
    cover_art_url    VARCHAR(500)             NOT NULL, -- Album/song cover image
    lyrics           TEXT                     NOT NULL,
    duration_seconds INT                      NOT NULL, -- Duration in seconds
    uploader_uid     BIGINT                   NOT NULL,
    creation_type    INT                      NOT NULL, -- 1=original, 2=remix/cover, 3=remix/cover for remix/cover
    play_count       BIGINT                   NOT NULL DEFAULT 0,
    like_count       BIGINT                   NOT NULl DEFAULT 0,
    is_private       BOOLEAN                  NOT NULL DEFAULT FALSE,
    release_time     TIMESTAMP WITH TIME ZONE NOT NULL,
    create_time      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    update_time      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_songs_uploader_uid ON songs (uploader_uid);
CREATE INDEX idx_songs_is_private ON songs (is_private);
CREATE INDEX idx_songs_created_time ON songs (create_time);
CREATE INDEX idx_songs_play_count ON songs (play_count DESC);
CREATE INDEX idx_songs_like_count ON songs (like_count DESC);

CREATE TABLE song_origin_info
(
    id             BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    song_id        BIGINT NOT NULL,
    origin_song_id BIGINT,
    origin_title   VARCHAR(255),
    origin_artist  VARCHAR(255),
    origin_url     VARCHAR(500)
);

-- Production crew - producers, composers, etc.
CREATE TABLE song_production_crew
(
    id          BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    song_id     BIGINT       NOT NULL,
    role        VARCHAR(100) NOT NULL, -- 'producer', 'composer', 'lyricist', 'mixer', etc.
    uid         BIGINT,                -- Null if the producer/composer is not registered in our platform
    person_name VARCHAR(255)
);

CREATE INDEX idx_song_production_crew_song_id ON song_production_crew (song_id);
CREATE INDEX idx_song_production_crew_uid ON song_production_crew (uid);
CREATE INDEX idx_song_production_crew_role ON song_production_crew (role);

CREATE TABLE song_external_links
(
    id       BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    song_id  BIGINT       NOT NULL,
    platform varchar(32)  NOT NULL,
    url      varchar(500) NOT NULL
);
CREATE INDEX idx_song_external_links_song_id ON song_external_links (song_id);

-- Song plays tracking for analytics
CREATE TABLE song_plays
(
    id            BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    song_id       BIGINT NOT NULL,
    user_id       BIGINT, -- NULL for anonymous plays
    anonymous_uid BIGINT, -- Unique identifier for anonymous users
    play_time     TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);
CREATE INDEX idx_song_plays_song_id ON song_plays (song_id);
CREATE INDEX idx_song_plays_user_id ON song_plays (user_id);
CREATE INDEX idx_song_plays_play_time ON song_plays (play_time);

-- Song likes
CREATE TABLE song_likes
(
    song_id     BIGINT NOT NULL,
    user_id     BIGINT NOT NULL,
    create_time TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    PRIMARY KEY (song_id, user_id)
);
CREATE INDEX idx_song_likes_song_id ON song_likes (song_id);
CREATE INDEX idx_song_likes_user_id ON song_likes (user_id);

-- Song tags - tag definitions
CREATE TABLE song_tags
(
    id          BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    name        VARCHAR(100) UNIQUE      NOT NULL,              -- In default language (if there is no translation)
    description TEXT,                                           -- In default language (if there is no translation)
    is_active   BOOLEAN                  NOT NULL DEFAULT TRUE, -- True if the tag is merged, set to false
    create_time TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    update_time TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- Song tag references - many-to-many relationship
CREATE TABLE song_tag_refs
(
    song_id BIGINT NOT NULL,
    tag_id  BIGINT NOT NULL,
    PRIMARY KEY (song_id, tag_id)
);
CREATE INDEX idx_song_tag_refs_song_id ON song_tag_refs (song_id);
CREATE INDEX idx_song_tag_refs_tag_id ON song_tag_refs (tag_id);

