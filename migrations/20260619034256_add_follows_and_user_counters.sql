-- Following relationships
CREATE TABLE follows (
    follower_id BIGINT NOT NULL,
    followed_id BIGINT NOT NULL,
    create_time TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_follows_follower_time
    ON follows (follower_id, create_time DESC);
CREATE INDEX idx_follows_followed_time
    ON follows (followed_id, create_time DESC);
CREATE UNIQUE INDEX idx_follows_follower_followed
    ON follows (follower_id, followed_id);

-- Add follower_counts and following_count to users table
-- This is an antipattern to single responsibility principle, but it is a performance optimization to avoid counting followers and following on the fly.
ALTER TABLE users ADD COLUMN follower_count  BIGINT;
ALTER TABLE users ADD COLUMN following_count BIGINT;
