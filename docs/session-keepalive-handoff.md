# 会话保活交接

## 交付范围

分支 `feat/session-keepalive` 实现全局风险确认、逐账号开关、可选模型、唯一动态代理、手动与后台重写、失败重试、按账号与模型隔离的 State 缓存，以及诊断日志。使用流程和边界见 [设计说明](session-keepalive-design.md)，接口见 [API](api.md#会话-state-刷新)。

主要入口：

| 范围 | 文件 |
| --- | --- |
| 缓存、刷新、重试、调度 | `backend/crates/providers/openai/src/session_manager/manager.rs` |
| 重写诊断与脱敏 | `backend/crates/providers/openai/src/session_manager/diagnostics.rs` |
| 共用测试请求构造 | `backend/crates/providers/openai/src/transport/request.rs` |
| 动态代理隔离与测试准入 | Store 的 `postgres/proxies.rs`、`runtime_settings.rs` |
| 持久化 | `backend/migrations/0016_session_keepalive.sql` |
| 设置页风险确认 | `frontend/src/views/settings/components/SessionKeepaliveCard.vue` |
| 动态代理页面 | `frontend/src/views/proxies/` |
| 账户模型选择与刷新 | `frontend/src/views/accounts/components/AccountSessionModelsField.vue`、`AccountSessionStateModal.vue` |

## 本地验证

2026-09-18，macOS / Apple Silicon、Rust 1.97.0、Node 24.4.1、pnpm 11.7.0。使用专用 PostgreSQL 17（55439）与 Redis（56389），测试自行隔离 schema、数据库和 key 前缀。没有读取生产账号 Token，也没有调用真实上游模型。

Cargo 使用 `RUST_MIN_STACK=16777216 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`，数据库测试配置专用 `CPR_TEST_DATABASE_URL`、`CPR_TEST_REDIS_URL`，清除生产连接覆盖变量。

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --manifest-path backend/Cargo.toml --all -- --check` | 通过 |
| `cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings` | 通过 |
| `pnpm --dir frontend format:check` | 通过 |
| `pnpm --dir frontend build`（含类型检查） | 通过 |
| 迁移目录 `shasum -a 256 -c .frozen-sha256` | 17 个迁移通过，旧迁移未修改 |
| `git diff --check` | 通过 |
| `cargo test --manifest-path backend/Cargo.toml --workspace --test main --no-fail-fast --locked`，`RUST_TEST_THREADS=4` | 见下表，全量未全绿 |

| 测试目标 | 通过 | 失败 |
| --- | ---: | ---: |
| Gateway App（含架构边界） | 42 | 0 |
| gateway-admin | 173 | 0 |
| gateway-api | 369 | 0 |
| gateway-core | 362 | 0 |
| gateway-host | 77 | 0 |
| gateway-protocol | 63 | 0 |
| gateway-store（真实专用 PostgreSQL / Redis） | 242 | 0 |
| provider-openai | 682 | 8 |
| provider-xai | 406 | 0 |

合计 2416 通过、8 失败，没有跳过失败用例。8 个失败均与此前在未修改基线 `b689a772` 上复现的 OpenAI Transport 失败名称一致，涉及 macOS TLS ClientHello 与 WebSocket 时序；不将全量测试称为通过。具体名称与基线证据见 [后端验证记录](verification/session-keepalive/backend-results.txt)。

15 项 Session Manager 测试覆盖账户与模型隔离、三个自定义模型、真实本地 HTTP 双出口、TTL、在途失效、重复刷新、429 冷却、有限重试及 Retry-After、完整结构化诊断字段与嵌套认证脱敏。Store 测试覆盖单个动态代理限制、ID/URL/导入绑定拒绝、地址变化清除测试结果、全局默认关闭、测试准入、模型持久化与快照投影。Admin/API 测试覆盖风险确认、请求合同和错误边界。

模型 ID 与选择器修正后的增量验证：`cargo test -p provider-openai session_manager --locked` 15 项通过，`cargo test -p gateway-store session_ --locked` 13 项通过（含账号默认值/存量迁移 2 项）；相关 crate 严格 Clippy 与 Rustfmt 通过，前端 ESLint、类型检查与构建通过，18 个迁移冻结校验通过。存量迁移覆盖旧名称转换、转换后去重及自定义模型保留。首次扩大 Store 过滤范围时因测试 Redis 未启动、随后认证配置不匹配导致 10 项失败；修正专用测试实例配置后同一命令 13 项全过。上述全量结果属于此前功能验证，本次未重跑全量。

## 浏览器证据

使用 Vite、实际 Vue 页面与本机 Chrome 无头浏览器，API 以合成数据拦截。验证动态代理保存后测试失败及再次测试成功、全局默认关闭、风险弹窗取消与确认、账户选择四个模型（目录勾选＋手动添加）、业务代理下拉排除动态代理、手动刷新等待与部分成功、再次刷新成功、缺失代理错误。另验证默认模型完整 ID、目录重复项去重、已选模型不出现在可添加列表、取消后重新添加，以及手动输入已有模型不重复。没有页面脚本异常。

四个模型仅为验收样例；产品支持每账号 1～32 个。检查桌面 1440×1100 浅色及 390×844 深色，窄屏无横向溢出。截图均为合成账号和无效示例地址：

- [动态代理表单](verification/session-keepalive/dynamic-proxy-form.png)、[测试失败](verification/session-keepalive/dynamic-proxy-failed.png)、[代理列表](verification/session-keepalive/dynamic-proxy-list.png)
- [风险确认](verification/session-keepalive/risk-confirmation.png)、[设置页](verification/session-keepalive/settings.png)
- [可添加模型](verification/session-keepalive/account-available-models.png)、[窄屏模型选择](verification/session-keepalive/account-models-mobile-dark.png)
- [账户模型](verification/session-keepalive/account-models.png)、[桌面刷新结果](verification/session-keepalive/partial-desktop.png)、[窄屏深色](verification/session-keepalive/partial-mobile-dark.png)

这些结果不替代浏览器到真实后端、代理、上游的完整验收。

## 部署与真实业务验收

1. 应用迁移后，全局功能仍关闭。到代理管理保存一个动态代理并测试。
2. 在设置页确认风险、开启并保存。为选定的 OpenAI OAuth 账户开启重写，按实际账户目录选择上游模型 ID。
3. 在账户页手动刷新，核对逐模型结果；失败会自动重试，最终错误通过重写 ID 对应 `session_keepalive` 日志。日志包含 State 与请求/响应字段，其他认证内容脱敏；禁止把真实日志放入提交或公开报告。
4. 用原客户端验证连续工具调用、原生续写、换号重试、HTTP 和复用 WS 连接；确认业务继续走原出口、State 能在真实双出口间使用。
5. 观察至少一轮后台更新及 TTL，确认配额、费用和限流影响。模型较多时手动刷新等待更长，部署反向代理超时必须匹配管理请求；每模型最多 3 次 30 秒请求和 2 次至多 30 秒退避，前端等待上限覆盖 32 个模型。

尚未验证真实上游接受的模型 ID、State 实际寿命、跨出口可用性或业务收益。官方协议偏离见设计说明。代理连通测试只证明代理可连通，不证明上游一定接受重写。轮间等待、重试和规模限制不能保证缓存始终有效。

## 停用与回退

关闭全局开关并保存，停止新业务请求的 State 覆盖及后续重写；关闭单个账号仅停用该账号。已发出的请求无法撤回。网关进程重启可恢复 Redis 中未过期且凭据版本一致的 State；Redis 数据丢失则重新获取，期间缺票的受管理账号／模型暂停承接业务请求。

程序回退前先关闭全局和账号开关。旧二进制可能拒绝包含未知迁移的数据库；使用保留新迁移的回退构建，或在恢复环境还原匹配旧版本的备份并验收，不删除迁移记录或修改旧 checksum。
