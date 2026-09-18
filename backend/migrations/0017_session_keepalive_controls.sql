-- 全局保活默认关闭，旧账号仅保留选择，须在设置页重新确认后启用。
ALTER TABLE runtime_settings ADD COLUMN session_keepalive_enabled BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE provider_accounts ADD COLUMN session_keepalive_models TEXT[] NOT NULL DEFAULT ARRAY['5.6 sol', '6'];

-- 动态代理仅供探活；唯一索引防止并发创建第二个全局出口。
ALTER TABLE outbound_proxies ADD COLUMN is_dynamic BOOLEAN NOT NULL DEFAULT FALSE;
CREATE UNIQUE INDEX outbound_proxies_single_dynamic ON outbound_proxies (is_dynamic) WHERE is_dynamic;

-- 旧 oam_proxy 保留供升级回退，不再参与运行时出口选择。
-- 代理 URL 需由领域解析器规范化，不能在 SQL 中复制未经规范化的旧地址而绕过业务隔离。
