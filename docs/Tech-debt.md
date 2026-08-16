# 待办 / 待优化清单

> 记录代码审查与日常开发中发现的待解决 / 待优化点。每一条含：现状、位置、问题、建议、优先级。
> 修复后请从本文档移除对应条目，避免残留已过期项。

## 代码重复

### 1. sqlite 仓储的 parse_utc / db_err / bad_row 三份重复

- **位置**：`src-tauri/src/infrastructure/sqlite/{channel,api_key,request_log}.rs`（各有一份，约 30 行重复）
- **问题**：同一套"解析 RFC3339 / 包装 sqlx 错误 / 构造坏行错误"逻辑复制了三次，改一处易漏两处。
- **建议**：收敛到共享位置（如 `infrastructure/sqlite.rs` 内 `pub(crate) fn` 或独立 `sqlite/error.rs`），三个仓储复用。
- **优先级**：中

### 2. ChannelType 字符串↔枚举转换缺权威定义

- **位置**：`domain/channel.rs:19` 定义 `ChannelType` 枚举，但 `to_str` / `from_str` 落在 `infrastructure/sqlite/channel.rs:156,166`
- **问题**：domain 层枚举没有 `Display` / `FromStr`，转换逻辑外置在 infrastructure；若 providers 等其他层也要转换就得复制或上移，且 infra 层依赖的字符串集合缺少编译期约束。
- **建议**：在 domain 层为 `ChannelType` 实现 `Display` / `FromStr`，infrastructure 复用。
- **优先级**：低-中

### 3. 时间格式不统一：fmt_time 固定 9 位 vs to_rfc3339 可变精度

- **位置**：`sqlite/request_log.rs:210` 用 `fmt_time`（固定 9 位纳秒，为对齐日期区间比较）；`sqlite/channel.rs` / `sqlite/api_key.rs` 用 `.to_rfc3339()`（可变精度）
- **问题**：可变精度在子秒日志与整秒边界比较时会错位（request_log 特意修了这个坑，另两处没修）。当前 channel/api_key 的 `created_at` 不参与区间比较，未踩雷，但属隐患。
- **建议**：统一用固定精度格式（或在 infra 层共享一个 `fmt_time`），与 §1 一起收敛。
- **优先级**：低

## 潜在优化

### 4. 统计聚合：每请求全表扫描 + O(7n) 内存过滤

- **位置**：`usecases/stats.rs:87` `repo.stat_rows(None, None)` 拉全量轻量行，随后 `:106`、`:119` 对全量做 7 次 `filter`（今天 1 次 + 7 天趋势各 1 次）
- **问题**：单用户本地 SQLite、日志量数千~数万条时无问题；若长期累计几十万条以上，每次拉仪表盘都是 O(7n)，性能与内存开销随日志量线性增长。
- **建议**：届时再把聚合下推到 SQL（GROUP BY 时间截断），或只拉 `[today-6 天, now]` 窗口内的行；当前不建议动。
- **优先级**：中（触发条件：日志量达到明显拖慢仪表盘时）

### 10. 渠道调度缺 cooldown（坏渠道每次请求浪费一次尝试）

- **位置**：`src-tauri/src/domain/dispatcher.rs`（当前为无状态「优先级分组 + 组内权重随机」，无失败记忆）
- **问题**：坏渠道在队首被打中时，每次请求都会浪费一次尝试后才落到健康渠道；单机单用户可接受（现有 retry 已兜住），但若某渠道持续失败，代价随请求量线性放大。
- **建议**：等有失败 metrics 后（见 `docs/design/backend/Loadbalancing-reference.md` 可演进路线第 5 条）做轻量内存 cooldown——记录每渠道连续失败次数与 `cooldown_until`，429/5xx/timeout 达阈值后短时移出候选池，成功清零。本次路由改动已明确推迟该能力。
- **优先级**：低（触发条件：观察到某渠道持续失败、每次请求都在空转时）

## 文档

### 5. Architecture-backend.md 提到 utils/ 但实际已删除

- **位置**：`docs/Architecture-backend.md:82-84` 目录树写了 `utils.rs` / `utils/`（hex 编码）；`src-tauri/src/` 下不存在该目录
- **问题**：文档与代码不符，易误导。
- **建议**：从架构文档删除 utils 目录条目（或补上实际现状）。
- **优先级**：低

## 重构建议

### 7. 集中 ID 生成：新增 id_generator.rs

- **位置**：主键 `Uuid::now_v7()` 分散在各 usecases 构造处（`usecases/api_key.rs:47`、`usecases/channel.rs:53`、`usecases/proxy.rs:353`、`usecases/auth.rs:66`）；`generate_key()`（`sk-revue-<16 hex>`）为私有函数在 `usecases/api_key.rs:149`；trace_id 直接 `Uuid::now_v7().to_string()` 在 `http/handlers.rs:60`；测试样本的 hex 生成又复制了一份在 `test_support.rs:95`
- **问题**：ID 生成逻辑无单一归属——sk-revue 前缀生成藏在 usecase 内，测试侧又复制了类似 hex 生成；trace_id / 主键生成散落各处。
- **建议**：新增 `src-tauri/src/utils/id_generator.rs`（snake_case，遵循 Rust 命名；按 2024 Edition 布局需同步建 `utils.rs` 模块入口），集中 `now_id()` / `generate_api_key()` / `new_trace_id()`；`generate_key` 的 sk-revue 逻辑迁入。注意：`test_support.rs:93` 用 v4 hex 有明确原因（v7 前 12 位是毫秒时间戳，同毫秒调用会碰撞导致 UNIQUE 冲突），迁入时保留该差异，勿一律换成 v7。
- **优先级**：低-中

### 8. 统一时间格式化工具函数：fmt_utc / parse_utc

- **位置**：`sqlite/request_log.rs:210`（`fmt_time`，固定 9 位纳秒）；`sqlite/{channel,api_key,request_log}.rs` 各一份 `parse_utc`；`channels` / `api_keys` 写入用可变精度 `.to_rfc3339()`
- **问题**："ISO 8601 UTC 字符串存储（固定精度 → 字符串可排序）"纪律只在 `request_logs` 落地，另两张表靠"不参与时间比较"侥幸安全；缺权威工具函数，新代码容易退回 chrono 默认可变精度，把"可排序"前提悄悄破坏掉。
- **建议**：在 `infrastructure/sqlite.rs` 加一对共享函数 `fmt_utc(dt) -> String`（固定 9 位纳秒）与 `parse_utc(s)`，三个仓储改用它——让"每张表的字符串都可排序"由函数保证而非代码作者自觉。`day_start_utc`（`usecases/stats.rs:145`）仅 stats 用，不挪。与 §1、§3 一并处理。
- **优先级**：中

## 非缺陷记录（仅备忘）

### 6. handlers.rs 测试直接 sqlx::query 打 pool

- **位置**：`interface/http/handlers.rs:428,463,553,663`（均在 `#[cfg(test)]` 内）
- **说明**：测试断言直接用 `sqlx::query` 验证落库结果，绕过了仓储层。属测试便利，无安全问题（均为无参数查询）；如希望测试只经仓储断言，可改，非必须。

### 9. 一些我发现的待讨论点

- `src-tauri\src\domain\provider.rs` 中的 `Usage` struct，改名为 `TokenUsage`
- `src-tauri\src\infrastructure\providers` 下各个 `Adaptor` Trait 实现中的 `default_models()` 方法是否需要修改？因为我发现都是硬编码的模型名称。我认为可以参考 cc-switch 的方式，给一个按钮“获取模型列表”来通过请求获取所有可使用的模型，而不是直接硬编码。
- 增加 deepseek 适配器实现，在 `src-tauri\src\infrastructure\providers` 下。由于 DeepSeek 的 API 完全兼容 OpenAI 协议，所以参考 `src-tauri\src\infrastructure\providers\openai.rs` 来写
- 是否要增加“circuit 模块”
