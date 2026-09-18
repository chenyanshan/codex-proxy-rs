-- State 重写在前三轮单探针之后采用配置并发与轮间等待。
ALTER TABLE runtime_settings
    ADD COLUMN session_rewrite_concurrency BIGINT NOT NULL DEFAULT 3
        CHECK (session_rewrite_concurrency BETWEEN 1 AND 10),
    ADD COLUMN session_rewrite_retry_interval_seconds BIGINT NOT NULL DEFAULT 6
        CHECK (session_rewrite_retry_interval_seconds BETWEEN 1 AND 300);
