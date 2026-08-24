# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-08-24

### Added

#### 中文

- 新增渠道管理，支持创建、编辑、删除渠道，并将数据持久化到 SQLite
- 新增渠道启停、优先级、权重和模型映射字段，并支持表单录入与回显
- 新增 OpenAI、DeepSeek、Custom、Claude 和 Gemini 五类渠道类型
- 新增管理界面骨架，包含侧边栏、顶栏，以及 Dashboard、Channels、API Keys、Logs、Settings 页面路由
- 新增服务状态指示器，可根据 `server-started` 和 `server-stopped` 事件实时更新数据面运行状态
- 新增浅色、深色、跟随系统三态主题切换，其中 `system` 由 CSS media query 响应

#### English

- Add channel management for creating, editing, and deleting channels with SQLite persistence
- Add channel enablement, priority, weight, and model mapping fields with form round-trip display
- Add selectable provider types for OpenAI, DeepSeek, Custom, Claude, and Gemini channels
- Add the management UI shell with sidebar, top bar, and Dashboard, Channels, API Keys, Logs, and Settings routes
- Add a service status indicator that reflects `server-started` and `server-stopped` events in real time
- Add a three-state theme selector for light, dark, and system themes, with `system` handled by CSS media queries
