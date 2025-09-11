-- Add migration script here
CREATE TABLE version
(
    id             BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    version_name   VARCHAR(255)             NOT NULL,
    version_number INT                      NOT NULL,
    changelog      TEXT                     NOT NULL,
    variant        VARCHAR(32)              NOT NULL,
    url            TEXT                     NOT NULL,
    release_time   TIMESTAMP WITH TIME ZONE NOT NULL,
    create_time    TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    update_time    TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX version_variant_idx ON version (variant, release_time DESC);