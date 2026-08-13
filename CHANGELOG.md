# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- 新增渠道管理：创建 / 编辑 / 删除渠道，数据持久化到 SQLite，界面操作与数据库一致
- 新增渠道启停开关与优先级 / 权重 / 模型映射字段的录入与回显
- 新增 5 类渠道类型（OpenAI / DeepSeek / Custom / Claude / Gemini）可选
- 新增管理界面骨架：侧边栏 + 顶栏 + 五个页面路由（Dashboard / Channels / API Keys / Logs / Settings）
- 新增服务状态灯：顶栏随 `server-started` / `server-stopped` 事件实时反映数据面运行状态
- 新增主题三态切换：浅色 / 深色 / 跟随系统（`system` 由 CSS 媒体查询响应，零 JS 监听）

## [x.y.z] - YYYY-MM-DD

### Added

- Add feature

### Changed

- Change existing behavior

### Deprecated

- Mark feature as deprecated for future removal

### Removed

- Remove feature

### Fixed

- Fix bug

### Security

- Fix security vulnerability
