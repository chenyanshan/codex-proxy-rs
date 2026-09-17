# 会话保活交接

## 交付范围

实现五个步骤及账户页面手动刷新：默认关闭的账号开关、全局运维代理、按账号和精确模型隔离的内存缓存、独立探活 Client、后台抖动调度、业务 State 覆盖和管理员 API。无需先确认故障是否由 State 引起，管理员按需要开启。

代码位于功能分支 `feat/session-keepalive`，基线为 `b689a772`（v3.10.0 代码加设计文档）。主要入口：

| 范围 | 文件 |
| --- | --- |
| 缓存、探活、调度和拦截 | `backend/crates/providers/openai/src/session_manager.rs` |
| Provider 装配与业务挂载 | `backend/crates/providers/openai/src/lib.rs`、`provider/mod.rs`、`provider/workers.rs`、`admin.rs` |
| 持久化 | `backend/migrations/0016_session_keepalive.sql`、Store 的 `runtime_settings.rs` 和 `provider_accounts/` |
| 管理端口与 wire | Admin 的账号/设置模型和用例、API 的 `admin/accounts/` 与 `admin/settings.rs` |
| 设置页 | `frontend/src/views/settings/components/SessionKeepaliveCard.vue` |
| 手动刷新 | `frontend/src/views/accounts/components/AccountSessionStateModal.vue`、`composables/useAccountSessionState.ts` |
| 行为测试 | `backend/crates/providers/openai/tests/session_manager.rs`；Store 与 API 对应测试目录 |

完整行为、网络边界和 API 合同分别见 [设计说明](session-keepalive-design.md) 与 [API](api.md#会话-state-刷新)。

## 本地验证环境

2026-09-18，在 macOS / Apple Silicon、Rust 1.97.0、Node 24.4.1、pnpm 11.7.0 上验证。PostgreSQL 17 使用本次创建的专用实例 `127.0.0.1:55439`，Redis 使用 `127.0.0.1:56389`；测试账号具有建库权限，测试自行隔离 schema、数据库与 key 前缀。未读取生产账号 Token，未请求真实上游模型。

Rust 验证设置 `RUST_MIN_STACK=16777216`。为降低本机磁盘占用，同时设置 `CARGO_INCREMENTAL=0`、`CARGO_PROFILE_DEV_DEBUG=0`、`CARGO_PROFILE_TEST_DEBUG=0`；没有修改仓库构建配置。初次构建缓存耗尽空间后，使用 `cargo clean` 清理本次产物再运行。

## 验证结果

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --all --manifest-path backend/Cargo.toml -- --check` | 通过 |
| `cargo clippy --manifest-path backend/Cargo.toml --all-targets --all-features --locked -- -D warnings` | 通过 |
| `pnpm --dir frontend format:check` | 通过 |
| `pnpm --dir frontend build`（含类型检查） | 通过 |
| `shasum -a 256 -c .frozen-sha256`（迁移目录） | 16 个迁移全部通过；旧迁移未变 |
| `git diff --check` | 通过 |
| `cargo test --manifest-path backend/Cargo.toml --workspace --test main --no-fail-fast --locked` | 见下表；全量未全绿，没有跳过失败用例 |

后端命令使用上文的编译环境变量，并配置专用 `CPR_TEST_DATABASE_URL`、`CPR_TEST_REDIS_URL`；清除可能覆盖测试环境的生产 `CPR_DATABASE_*` / `CPR_REDIS_*` 变量。数据库测试实际执行，包括全迁移和重新打开数据库。

| 测试目标 | 通过 | 失败 |
| --- | ---: | ---: |
| Gateway App（含架构边界） | 42 | 0 |
| gateway-admin | 172 | 0 |
| gateway-api | 369 | 0 |
| gateway-core | 362 | 0 |
| gateway-host | 77 | 0 |
| gateway-protocol | 63 | 0 |
| gateway-store（真实专用 PostgreSQL / Redis） | 241 | 0 |
| provider-openai（含新增 10 项） | 677 | 8 |
| provider-xai | 406 | 0 |

合计 2409 通过、8 失败，各目标均无 ignored。失败均在原有 OpenAI Transport 用例；独立检出未修改基线 `b689a772` 后，在相同 macOS 环境运行 Provider 全测试，667 通过、8 失败，**失败名称集合完全相同**。因此本次没有消除这些既有验证失败，也不将全量测试称为通过。TLS 失败发生在测试解析 ClientHello 的扩展 51，WebSocket 失败涉及模拟时间、握手和流超时；未进一步认定其根因。

复现失败用例见 [后端验证记录](verification/session-keepalive/backend-results.txt)。基线对照完成后，重新编译当前工作区并确认执行 685 项 Provider 测试，避免共用临时构建缓存造成结果混淆。

### 保活行为覆盖

新增 10 项 Session Manager 测试已通过，使用真实本地 HTTP 连接和模拟上游：

- 两个账号 × 两个模型分别刷新、注入，四个 State 完全隔离。
- 关闭开关、不支持的精确名称、缺失代理不探活或不覆盖。
- 部分失败保留旧值，到期后不再使用；缺失 Header 或 SSE 失败不缓存。
- 同账号重复刷新返回冲突；失效后的在途结果不能回填。
- 429 冷却阻止后续模型及重复刷新；凭据 revision 变化后旧缓存不注入。
- Worker 首轮后在 50–58 分钟范围内进入下一轮，休眠可取消。
- 两次探活只到运维代理；随后通过原有 `CodexBackendClient` 发出的业务 HTTP 请求携带更新后的 Header，只到业务代理。

Store 测试验证代理默认空、非法 URL 与脱敏、遗漏保留和显式清空，以及账号默认关闭、开启后保存其他字段仍保留、显式关闭。API 测试验证管理鉴权、账号 ID、未知字段拒绝、依赖不可用、配置序列化与脱敏。未知 JSON 字段沿用现有 422 合同。

### 浏览器验证

使用实际 Vue 页面、Vite 和本机 Chrome 无头浏览器，API 由合成数据拦截，**不是完整后端连通验收**。测试脚本位于本地临时目录；仓库按现有规则不新增前端测试套件。

已操作：设置代理并保存、确认密码输入样式、开关关闭时无手动入口、键盘切换开关并保存、手动刷新 loading、关闭禁用、防重复请求、双模型部分成功、再次刷新成功、代理缺失错误。页面没有脚本异常。桌面 1440×1100 浅色和 390×844 深色均检查截图，窄屏弹窗无横向溢出。

截图仅含合成账号与无效示例代理：

- [系统设置](verification/session-keepalive/settings.png)
- [桌面部分成功](verification/session-keepalive/partial-desktop.png)
- [窄屏深色部分成功](verification/session-keepalive/partial-mobile-dark.png)
- [窄屏代理缺失](verification/session-keepalive/proxy-missing-mobile.png)

## 真实业务接手步骤

1. 在单副本测试部署应用新迁移；确认旧迁移校验通过、新账号默认关闭。保留当前数据库备份，旧迁移及 `_sqlx_migrations` checksum 不要修改。
2. 在系统设置保存独立运维代理。运维 Client 按需求跳过证书校验，代理必须可信；账号业务代理仍按原配置使用。
3. 仅为选定的 OpenAI OAuth 测试账号开启会话保活，在账号操作菜单点击“刷新 State”，分别检查 `5.6 sol` 和 `6` 的结果。
4. 确认真实上游接受这两个精确模型名且返回可用 State。不要用定价表名称自行替代；若全局模型映射改变最终模型名，按设计精确匹配检查是否命中。
5. 从原客户端分别测试新轮次、连续工具调用、原生续写、换号重试、HTTP 和复用 WS 连接。使用两个网络出口的脱敏观测确认探活与业务分别走预期线路，核对实际 State 可跨出口使用。
6. 持续观察至少一轮后台刷新及 60 分钟到期，检查限流、延迟、配额与费用影响；刷新凭据、关闭开关及删除账号时确认后续请求停止使用旧缓存。不要记录 State 或 Token 原文来证明隔离；已有显式 `request_dump` 会保存完整业务 Header/正文，验收时保持关闭。

### 明确保留的缺口

- 未验证真实上游模型 ID、返回 Header、实际 State 寿命、跨出口可用性及业务收益。
- 已核验官方源码要求 State 不跨轮次使用；本实现是管理员显式选择的偏离，不是官方兼容实现。源码依据见设计说明；真实续写与工具调用风险需要业务验收。
- HTTP 双代理测试使用本地明文模拟网络；真实 HTTPS 代理、证书、DNS、出口路由与生产部署未验收。
- 新功能的 WS 验证覆盖请求逐帧 metadata 构造，真实连接复用后的跨轮次热更新尚无上游运行证据。
- 浏览器使用模拟 API；真实登录、浏览器到新 API 再到 Provider 的完整链路、部署反向代理超时需接手验证。单次手动刷新最多两个 30 秒请求，前端超时 70 秒。
- 配置与开关有持久审计；手动刷新只有内存更新和脱敏运维日志，没有独立持久审计事件或用户用量账单。后台周期可能因账号规模与限流导致缓存提前过期。

## 停用与恢复

立即停止某账号后续覆盖：关闭其“会话保活”并保存；该账号缓存失效，在途刷新结果不回填。已发出的业务请求不撤回。

全局清空 `oamProxy` 只停止后续探活，已有有效缓存会保留到原截止时间；需要全面停止覆盖时同时关闭已开启的账号。进程重启会清空所有内存 State，但持久开关不变，开启的账号会在启动后重新探活。

回退程序版本前先关闭账号开关。旧二进制可能因数据库包含未知迁移而拒绝启动，优先通过开关停用；如需程序回退，使用保留 `0016` 迁移的回退构建，或在恢复环境还原与旧版本匹配的备份后验收。不要删除迁移记录或修改旧 checksum 来绕过启动检查。
