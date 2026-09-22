# 前端优化 Spec

## Requirements

- 增强 ChannelsPage——渠道统计卡片 + 拖拽排序
- 增强 ApiKeysPage——配额环形图 + 使用进度
- 增强 LogsPage——日志过滤/搜索 + 详情展开
- 增强 DashboardPage——6 指标卡片 + 多协议统计
- 增强 UsagePage——多协议（OpenAI/Anthropic/Responses）用量分析
- 实现 UpdateChecker——自动检测版本更新
- 实现 Migration 005/006/007——response_choices/seq/trace_id

### ChannelsPage

拖拽排序：考虑先使用原生 HTML5 Drag and Drop API。是否要引入 react 第三方拖拽库

### ApiKeysPage

### LogsPage

### DashboardPage

### UsagePage

### UpdateChecker

### Migration

## 代码参考

直接参考 WaliAPI 项目的 branch 3-7-frontend-upgrade
