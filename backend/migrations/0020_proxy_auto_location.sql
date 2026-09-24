-- 手动位置与自动检测结果独立保存，关闭自动模式可恢复原有配置。
alter table outbound_proxies
    add column auto_location boolean not null default false,
    add column detected_location_json jsonb,
    add column last_location_detection_json jsonb not null default '{"status":"notRequested"}'::jsonb;
