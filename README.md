# revue-gate

A local LLM API gateway for the desktop. Aggregate multiple upstream providers (OpenAI, Claude, Gemini, DeepSeek, custom OpenAI-compatible) behind a single OpenAI-compatible endpoint — then point any AI client at `http://127.0.0.1:3000` and let the gateway handle auth, channel selection, model mapping, forwarding, quota accounting, and request logging.

> **Status:** early development (v0.1.0 MVP), features landing in stages. See [CHANGELOG.md](CHANGELOG.md) and [docs/roadmap.md](docs/roadmap.md).

**[中文说明](README.zh-CN.md)**

## Why

Running a handful of AI tools usually means juggling N provider keys, N API formats, and zero visibility into what actually gets sent. revue-gate puts one local gateway in front of your providers:

- **One OpenAI-compatible API** — `/v1/chat/completions` (stream + non-stream), `/v1/models`, `/health`. ChatBox, NextChat, OpenAI SDKs, and CLIs all point at the local address and just work.
- **Upstream keys never reach your clients.** Downstream only ever sees a local key (`sk-revue-…`); the real provider keys live in the gateway.
- **Channel management** — OpenAI, DeepSeek, and custom OpenAI-compatible endpoints pass through directly; Claude and Gemini go through protocol adapters. Set priority, weight, and per-model mapping.
- **Quota + audit** — per-key quota limits, a full request log (model, tokens, latency, error, trace id), and a dashboard for usage and channel health.
- **Local & private** — single user, data in local SQLite, no account system, no telemetry.

## Features (v0.1.0 MVP)

- **Data plane** — non-streaming and SSE streaming chat completions, with retry across candidate channels on failure
- **Channels** — CRUD, enable/disable, reorder, connectivity test, model mapping (5 built-in types)
- **API keys** — `sk-revue-<hex>` generation, Bearer auth, quota limits (`429` when exhausted)
- **Request logs** — full record, pagination, keyword / key / channel / model / date filters, detail view
- **Dashboard** — request & token counts, average latency, channel availability, 7-day trends
- **Settings** — host/port, light / dark / system theme, tray + autostart, retry policy

## Quickstart

Prerequisites: Node.js + pnpm, a stable Rust toolchain (pinned in `src-tauri/rust-toolchain.toml`), and the [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform (WebView2 on Windows).

```bash
pnpm install     # install frontend dependencies
pnpm tauri dev   # launch the desktop app (the data plane starts with it)
```

The gateway listens on `127.0.0.1:3000` by default (configurable in Settings; `port: 0` picks a random port).

## Usage

Create a channel (add your provider key) and an API key in the UI, then point any OpenAI-compatible client at the gateway:

```bash
curl http://127.0.0.1:3000/v1/chat/completions \
  -H "Authorization: Bearer sk-revue-<your-local-key>" \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4o-mini", "messages": [{"role": "user", "content": "Hello!"}]}'
```

Or any OpenAI SDK:

```ts
const client = new OpenAI({
  baseURL: "http://127.0.0.1:3000/v1",
  apiKey: "sk-revue-<your-local-key>",
});
```

Also available: `GET /health` → `{"status":"ok"}`, and `GET /v1/models` (models from all enabled channels, deduplicated).

## Documentation

- [Requirements](docs/Requirements.md) — product spec, MVP scope, architecture overview
- [Backend architecture](docs/Architecture-backend.md) · [Frontend architecture](docs/Architecture-frontend.md)
- [Roadmap](docs/roadmap.md) — v0.2.0 (security audit center) and v0.3.0 (RAG / MCP / more endpoints) plans

## Development

Checks are wired into git hooks via `prek` (configured in `prek.toml`) — run `prek install` once. They cover `cargo fmt`, `cargo check`, `cargo clippy -D warnings`, `cargo nextest run`, `tsc`, and the frontend build. Run them directly with:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo nextest run --manifest-path src-tauri/Cargo.toml
pnpm build   # tsc + vite build
```

## Security

The gateway keeps upstream provider keys out of downstream reach — clients authenticate with local-only keys. If you find a security issue, report it privately to the maintainer instead of opening a public issue.

## Contributing

The project is still in private development; bug reports and feature ideas via issues are welcome. Contribution guidelines will be published when the project opens up.

## License

Licensed under the Apache License, Version 2.0 (the "License"); see [LICENSE](LICENSE) for the full text.
