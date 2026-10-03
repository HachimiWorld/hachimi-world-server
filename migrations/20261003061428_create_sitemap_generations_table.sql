-- One row per sitemap generation, scheduled or triggered by hand.
-- A row left in 'running' means the server stopped during that generation.
CREATE TABLE sitemap_generations
(
    id           BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY NOT NULL,
    trigger_type TEXT                                            NOT NULL, -- 'schedule' | 'manual'
    status       TEXT                                            NOT NULL, -- 'running' | 'success' | 'failure'
    song_count   INT,
    file_count   INT,
    error        TEXT,
    start_time   TIMESTAMPTZ                                     NOT NULL,
    finish_time  TIMESTAMPTZ
);

CREATE INDEX idx_sitemap_generations_start_time ON sitemap_generations (start_time DESC);
