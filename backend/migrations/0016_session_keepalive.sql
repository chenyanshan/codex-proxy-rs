ALTER TABLE runtime_settings
    ADD COLUMN oam_proxy TEXT NOT NULL DEFAULT '';

ALTER TABLE provider_accounts
    ADD COLUMN enable_session_keepalive BOOLEAN NOT NULL DEFAULT FALSE;
