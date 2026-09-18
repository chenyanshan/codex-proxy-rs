# Codex 降智缓解实验版

`experimental/codex-anti-degradation` 基于 v3.10.0，用于探索 [#144](https://github.com/zyycn/codex-proxy-rs/issues/144)
反馈的 Codex 疑似风控与响应质量下降（“降智”）问题。实现沿用 PR #151 的 `X-Codex-Turn-State` 刷新、会话保活和动态代理。
它独立于 `main`，效果取决于上游行为，不承诺长期有效或进入正式版。
按需维护必要修复，不自动跟随主分支，也不承诺与主版本同步发布。

## 发布产物

实验版复用项目完整发布流程，平台清单与正式版一致：

| 平台 | 安装包 | Docker 镜像 |
| --- | --- | --- |
| Linux amd64 | tar.gz | 支持 |
| Linux arm64 | tar.gz | 支持 |
| macOS arm64 | tar.gz | — |

预发行页面同时提供 `compose.yaml`、`config.example.yaml`、`checksums.txt`。
安装包包含后端程序和管理端静态资源；镜像使用多架构标签，例如：

```text
ghcr.io/zyycn/codex-proxy-rs:3.10.0-exp.1
```

所有平台构建类型均为 `experimental`，禁用应用内一键更新；更新时手动选择指定预发行版本。
实验发行不会更新稳定版 `latest`。推送分支执行质量检查，发布时再统一构建全部平台产物。

## Docker 独立部署

必须使用独立安装目录、配置、PostgreSQL、Redis 与数据目录，不能与稳定版共用数据库或 `.runtime/`。
从指定预发行页面下载部署文件；以下示例安装 v3.10.0-exp.1：

```bash
mkdir -p codex-proxy-rs-codex-anti-degradation/deploy
cd codex-proxy-rs-codex-anti-degradation
export CPR_RELEASE_TAG='v3.10.0-exp.1'
curl -fsSL "https://github.com/zyycn/codex-proxy-rs/releases/download/${CPR_RELEASE_TAG}/compose.yaml" -o deploy/compose.yaml
curl -fsSL "https://github.com/zyycn/codex-proxy-rs/releases/download/${CPR_RELEASE_TAG}/config.example.yaml" -o deploy/config.example.yaml
curl -fsSL "https://github.com/zyycn/codex-proxy-rs/releases/download/${CPR_RELEASE_TAG}/checksums.txt" -o deploy/checksums.txt
(cd deploy && sha256sum --check --ignore-missing checksums.txt)
```

按照[手动安装](README.md#手动安装)创建 `.runtime` 目录、从当前版本的 `config.example.yaml` 创建配置并填写密码；
跳过下载正式版部署文件的步骤。然后启动：

```bash
docker compose -f deploy/compose.yaml config --quiet
docker compose -f deploy/compose.yaml pull
docker compose -f deploy/compose.yaml up -d --no-build --wait
```

实验发行附件已经合并隔离配置，使用独立 Compose 项目名，管理端为 `http://127.0.0.1:8081`，
数据库和 Redis 宿主端口分别为 `5433`、`6380`。镜像已固定到本次版本，不需要额外覆盖文件或镜像环境变量。

源码克隆中的 `deploy/compose.yaml` 仍是基础模板；源码部署需要与 `deploy/compose.experimental.yaml` 叠加，
设置 `CPR_EXPERIMENTAL_IMAGE` 为指定实验镜像。覆盖文件需要 Docker Compose 2.24.4 或更新版本。

## 二进制独立部署

下载与操作系统、架构对应的 `codex-proxy-rs_<版本>_<系统>_<架构>.tar.gz`，使用同一 Release 的 `checksums.txt` 校验，
解压到独立目录。从随包 `deploy/config.example.yaml` 创建配置，连接独立 PostgreSQL 和 Redis。
按照[部署与升级说明](README.md#镜像升级与源码构建)将 `api.asset_directory` 设置为 `../web/dist`，使用归档内的管理端资源。
同机同时运行稳定版时，为实验实例配置不同的监听端口。

功能默认关闭，配置方式与边界见[功能说明](../docs/session-keepalive-design.md)。

## 数据与回退

本分支在 v3.10.0 基线之上包含 `0016_session_keepalive.sql` 和 `0017_session_rewrite_retry_policy.sql`；后者只增加 State 重写的并发数与重试间隔，已有实验版 0016 数据库可以原地升级。稳定版基线 v3.10.0 使用 0001–0015；
实验库不能原地降级到稳定版，也不能随意合并主分支未来同编号迁移。
已经运行过 PR #151 原始 0016–0018 或旧版合并 0016 的源码实例，需要完整备份后重建与目标代码匹配的库，
并按目标表结构恢复业务数据；不要修改 `_sqlx_migrations` 的 checksum。

State 票据保存在该实验实例的 Redis 中，TTL 为 60 分钟；网关重启可恢复未过期票据，Redis 数据丢失则需要重新获取。启用 State 重写的账号／模型在缺票、过期或 Redis 不可用时暂停承接业务请求；升级后首次获取成功前也适用。请保持 Redis 数据卷与配置命名空间稳定，勿与其他实例混用。

首次试用优先新建空库，通过管理端导入所需账号。回到稳定版时停止实验实例，使用原稳定版实例或其试用前备份。
需要迁移试用期间新增数据时，单独导出业务数据并核对字段，不直接把实验库备份还原给稳定版。

## 维护与发布

按需挑选主分支修复，重新检查迁移编号、数据合同与实验功能；不要把整个实验分支合回 `main`。
发布新版本时，在实验分支维护 `release/notes.md`，使用现有 `release/publish <版本>` 入口，
后续版本采用 `3.10.0-exp.N` 形式，`N` 从 1 开始递增；只有实际同步新的正式版基线后，才调整前面的版本号。
现有已发布版本保留原标签、镜像与附件名称。

发布流程识别为实验构建和 GitHub Pre-release，全部检查通过后发布完整平台产物。
正文包含实验说明、本次变化、安装与使用、使用须知、反馈与致谢；工作流保留完整文案，
不重复追加安装章节或正式版基线的提交列表。发布准备、授权和验收按 [release 技能](../.agents/skills/release/SKILL.md)
及其[实验版流程](../.agents/skills/release/references/experimental.md)执行。
