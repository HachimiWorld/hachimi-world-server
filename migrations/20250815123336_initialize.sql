-- Users table with authentication support
CREATE TABLE users
(
    id              BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY (START WITH 100000),
    username        VARCHAR(50)              NOT NULL UNIQUE,
    email           VARCHAR(255)             NOT NULL UNIQUE,
    password_hash   VARCHAR(255)             NOT NULL,
    avatar_url      VARCHAR(500),
    bio             TEXT,
    gender          INT,
    is_banned       BOOLEAN                  NOT NULL DEFAULT FALSE,
    last_login_time TIMESTAMP WITH TIME ZONE,
    create_time     TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    update_time     TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_users_username ON users (username);
CREATE INDEX idx_users_email ON users (email);
CREATE INDEX idx_users_is_banned ON users (is_banned);

-- Refresh tokens table for JWT token management, and can be used for recording login behavior
CREATE TABLE refresh_tokens
(
    id             BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    user_id        BIGINT                   NOT NULL,
    token_id       VARCHAR(36)              NOT NULL UNIQUE,
    token_value    VARCHAR(2048)            NOT NULL,
    expires_time   TIMESTAMP WITH TIME ZONE NOT NULL,
    create_time    TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    last_used_time TIMESTAMP WITH TIME ZONE,
    device_info    VARCHAR(500),
    ip_address     VARCHAR(45),
    is_revoked     BOOLEAN                  NOT NULL DEFAULT FALSE
);
CREATE INDEX idx_refresh_tokens_user_id ON refresh_tokens (user_id);
CREATE INDEX idx_refresh_tokens_token_id ON refresh_tokens (token_id);
CREATE INDEX idx_refresh_tokens_expires_at ON refresh_tokens (expires_time);