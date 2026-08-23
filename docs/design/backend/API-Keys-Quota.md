# 密钥管理与配额控制

> 网关密钥（Virtual Key）管理与配额控制：下游用本地生成的密钥访问网关，上游真实 API Key 不出本地；每个密钥独立配额，超额返回 429。

## 为什么需要密钥

若下游直接使用上游 API Key，会导致：

- **密钥分散**：每个下游持有不同供应商的 key，难以统一管理。
- **难以管理**：撤销一个下游的访问要动上游 key，影响范围不可控。
- **难以统计**：无法按下游来源计量与限流。

revue-gate 的解法是 Virtual Key（`ApiKey` 实体）：每个下游应用一个独立的网关密钥，只在网关内部使用。

作用：

- **隔离**：注销某个密钥不影响其他应用。
- **计量**：每个密钥独立配额计数（`Quota` 值对象）。
- **限额度**：限制单密钥的最大 Token 消耗。
- **审计**：请求日志按 `api_key_id` 关联，每笔请求可追溯来源。
- **安全**：上游真实 Key 只存在本地 channels 表，下游只能拿到 Virtual Key。

## 设计思路

### Virtual Key 模型（domain/api_key.rs）

`ApiKey` 实体 = 唯一标识 + 描述 + 密钥 + 启用状态 + 配额：

| 字段 | 说明 |
| --- | --- |
| `id` | uuid v7（时间有序主键） |
| `name` | 管理员可读名称；创建 / 编辑时 trim，空白名拒绝 |
| `key` | 密钥明文 `sk-revue-*`，仅本地存储、仅创建时返回一次 |
| `enabled` | 启用 / 禁用；禁用后认证返回 401 |
| `quota` | `Quota { limit, used }`：`limit=None` 表示无上限，`used` 为已消耗额度 |

`Quota` 是值对象（只承载数据），**超限判定**由领域服务 `QuotaPolicy`（`domain/quota.rs`）负责，两者职责分离。

### 密钥格式

参考 OpenAI / Anthropic 的 `sk-` 前缀设计：

```
sk-revue-<16 位随机 hex>      # 共 25 字符，8 字节熵
```

- `sk`：secret key 标识。
- `revue`：产品标识，区分网关密钥与其他服务密钥。
- 16 位 hex：8 字节 OS 级随机源（`getrandom`）生成，碰撞概率可忽略；由领域函数 `generate_local_key()` 生成，**不支持外部指定**。

### 分层与依赖

依赖全部向内指向 domain，判定与结构在 domain、编排在 usecases、落库在 infrastructure：

| 分层 | 文件 | 职责 |
| --- | --- | --- |
| domain | `domain/api_key.rs` | `ApiKey` 实体 + `Quota` 值对象 + `ApiKeyRepository` trait + `generate_local_key` |
| domain | `domain/quota.rs` | `QuotaPolicy`：纯判定超限，无 DB / HTTP |
| usecase | `usecases/auth.rs` | `AuthenticateRequestUsecase`：鉴权 + 配额检查（401 / 429） |
| usecase | `usecases/api_key.rs` | 密钥 CRUD 编排 + `AccumulateUsageUsecase`（累加 used） |
| usecase | `usecases/proxy.rs` | 主闭环入口调鉴权；转发成功后累加配额（best-effort） |
| infrastructure | `infrastructure/sqlite/api_key.rs` | `SqliteApiKeyRepository`：trait 的 sqlx 实现 |
| interface（控制面） | `interface/commands/api_key.rs` | Tauri Commands：list / create / update / delete / set_enabled |
| interface（数据面） | `interface/http/handlers.rs` + `usecases/proxy.rs` | Bearer 提取 → 鉴权 → 转发 → 记账 |

### 配额控制

**判定（每次请求进入时）**——`QuotaPolicy::check`（`domain/quota.rs:22`）：

- `limit=None` → 永远允许。
- `limit=Some(n)` 且 `used >= n` → 超限（**边界值本身算超限**，`used == limit` 即 429）；`used < n` 允许。

**累加（请求成功后）**——`AccumulateUsageUsecase`（`usecases/api_key.rs:117`）：

- 按 usage 的 total tokens **saturating 累加**并持久化；累加可越过 limit（不静默截断），是否超限交给下次请求进入时的鉴权判定。
- 时机分两类：
  - **非流式**：转发成功后按完整 usage 累加一次。
  - **流式**：usage 随帧累积，只能在流结束（正常或出错）时按已聚合 usage 累加一次（`wrap_stream_bookkeeping`，详见 [HTTP_Server&SSE.md](./HTTP_Server&SSE.md)）。
- **best-effort**：上游已处理请求后，记账失败只 `warn` 不回 5xx（回 5xx 会导致下游重试 → 双重计费）。

### 认证链路（数据面）

```
handler 提取 Authorization: Bearer <key>
  → 缺 Bearer？→ 401（先于一切）
  → AuthenticateRequestUsecase（usecases/auth.rs:28）
      ① find_by_key：查不到 → 401
      ② enabled?：禁用 → 401
      ③ QuotaPolicy::check：超限 → 429
      ④ 放行，返回 ApiKey（供记账 / 日志关联 api_key_id）
```

## 对外接口

**数据面**：下游以 `Authorization: Bearer <virtual key>` 访问 `/v1/chat/completions`、`/v1/models`，与 OpenAI 协议一致。

**控制面**（Tauri Commands，前端经 `invoke` 调用）：

| 命令 | 行为 |
| --- | --- |
| `list_api_keys` | 列出全部密钥（按 name 升序，key 脱敏） |
| `create_api_key` | 创建新密钥，**唯一一次**返回明文 |
| `update_api_key` | 编辑 name / quota_limit / enabled（**密钥本身不可改**） |
| `delete_api_key` | 删除；不存在返回 NotFound |
| `set_api_key_enabled` | 启用 / 禁用切换 |

## 表结构（migrations/001_init.sql）

```sql
CREATE TABLE api_keys (
    id          TEXT PRIMARY KEY NOT NULL,   -- uuid v7
    name        TEXT NOT NULL,
    key         TEXT NOT NULL UNIQUE,        -- sk-revue-<16 位随机 hex>
    enabled     INTEGER NOT NULL DEFAULT 1,
    quota_limit INTEGER,                     -- NULL = 无上限
    quota_used  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
```

## 安全红线

- **明文只出现一次**：`create_api_key` 是唯一返回明文的路径；list / update / set_enabled 统一用 `sk-revue-••••••••••••••••` 占位脱敏（`interface/commands/api_key.rs:19`），任何控制面路径不得向前端回显明文。
- **上游密钥不出本地**：channels 表的 `api_key` 只用于本进程内 reqwest 上游请求，下游永远拿不到。
- **密钥不进日志**：`request_logs` 只存 `api_key_id`，统计与审计靠它关联，不写明文。

## 测试

- **domain/quota.rs**：无上限永远允许、未超限允许、**边界值（used == limit）算超限**。
- **usecases/api_key.rs**：密钥格式（前缀 + 16 位 hex + 25 字符）、生成去重、空名拒绝、更新不改变密钥且 upsert 不新增行、NotFound、删除、启停切换、累加持久化、**累加可越过上限（不截断）**。
- **usecases/auth.rs**：missing / unknown / disabled → 401，valid → 放行，超限 → 429，无上限永远允许。
- **command 层**：`mask_key` 脱敏后 key 恒为占位符。
