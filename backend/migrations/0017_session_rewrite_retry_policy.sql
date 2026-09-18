-- State 重写从首轮即采用配置并发，失败后按配置间隔持续重试。
ALTER TABLE runtime_settings
    ADD COLUMN session_rewrite_concurrency BIGINT NOT NULL DEFAULT 3
        CHECK (session_rewrite_concurrency BETWEEN 1 AND 10),
    ADD COLUMN session_rewrite_retry_interval_seconds BIGINT NOT NULL DEFAULT 2
        CHECK (session_rewrite_retry_interval_seconds BETWEEN 1 AND 300);
