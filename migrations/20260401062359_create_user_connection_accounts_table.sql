CREATE TABLE user_connection_accounts
(
    user_id               BIGINT      NOT NULL,
    provider_type         TEXT        NOT NULL,
    provider_account_id   TEXT        NOT NULL,
    provider_account_name TEXT        NOT NULL,
    public                BOOLEAN     NOT NULL,
    create_time           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    update_time           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_user_connection_accounts_user_id_provider_type ON user_connection_accounts (user_id, provider_type);