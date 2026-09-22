# ADR 0003: Multi-Protocol Conversion Engine

- **Status**: proposed
- **Date**: 2026-08-27
- **Authors**: Orangex-position0 / Codex

## Context

revue-gate currently exposes an OpenAI-compatible data-plane endpoint and routes canonical chat requests through `ProxyRequestUsecase`, which owns authentication, quota checks, channel selection, security audit, retry, billing, and request logging. Provider-specific differences are isolated behind `ProviderAdaptor` implementations in `infrastructure/providers/`.

The next compatibility goal is to let clients call the local gateway through multiple downstream protocol dialects:

- OpenAI Chat Completions via `/v1/chat/completions`.
- Anthropic Messages via `/v1/messages`.
- OpenAI Responses via `/v1/responses`.

The reference project `waliapi` separates provider/channel adapters from protocol codecs. Its later implementation also adds endpoint-aware route planning, native/conversion groups, codec versions, conversion reports, and SSE conversion. revue-gate should adopt the boundary idea without taking on the full route-plan rewrite in the first version.

## Decision

We will introduce a downstream protocol conversion layer that translates client-facing protocols into and out of an internal **Canonical Chat Protocol** based on OpenAI Chat Completions.

Specifically:

- Add `src-tauri/src/protocol.rs` and `src-tauri/src/protocol/` as a pure downstream protocol conversion module.
- Use Rust 2024-style same-name module files, not `mod.rs`.
- Keep `ProxyRequestUsecase` unchanged as the canonical chat orchestration boundary.
- Keep `infrastructure/providers/*` as the provider adaptation boundary for upstream HTTP, headers, API keys, provider errors, and usage parsing.
- Allow provider adapters to reuse pure mapping helpers from `protocol/`, but do not move provider HTTP orchestration into `protocol/`.
- Add a thin `CodecRegistry` that dispatches conversion functions; do not introduce a dynamic codec plugin system in the first version.
- Route `/v1/chat/completions` through the registry as an OpenAI Chat passthrough codec.
- Add `/v1/messages` and `/v1/responses` as downstream protocol handlers using a shared private `handle_protocol_request(...)` function.
- Support `Authorization: Bearer` on all endpoints; additionally support `x-api-key` only on `/v1/messages`.
- Introduce `ProtocolError`; do not fold protocol conversion failures into `ProxyError`.
- Format all `/v1/messages` errors as Anthropic-style error bodies.
- Use `serde_json::Value` at protocol boundaries and small internal structs for fields that affect conversion behavior.
- Use a permissive conversion strategy with `ConversionReport`, but forbid silent field dropping.
- Keep `ConversionReport` in memory in the first version; make it serializable for future request-log storage.
- Run security audit and request logging on the converted canonical body, not on the raw downstream body.

The intended code shape is:

```text
src-tauri/src/protocol.rs
src-tauri/src/protocol/
  types.rs
  registry.rs

  openai_chat.rs
  openai_chat/
    passthrough.rs

  anthropic_messages.rs
  anthropic_messages/
    request.rs
    response.rs
    stream.rs

  openai_responses.rs
  openai_responses/
    request.rs
    response.rs
    stream.rs

  mcp.rs
```

The request flow is:

```text
Client
  -> interface/http
  -> protocol/                  // downstream protocol conversion
  -> Canonical Chat Protocol
  -> usecases/proxy.rs           // auth, quota, routing, audit, retry, logs
  -> infrastructure/providers/   // provider adaptation
  -> Upstream Provider
```

The first version will support:

- `/v1/messages`: text blocks, custom tools, tool results, `system`, `max_tokens`, `stream`, `temperature`, `top_p`, `stop_sequences`, and basic `tool_choice`.
- `/v1/responses`: `model`, text `input`, `instructions`, `stream`, `temperature`, `top_p`, `max_output_tokens`, function tools, and basic `tool_choice`.

The first version will reject unsupported platform/stateful features with 400 instead of silently downgrading them. Examples include Anthropic multimodal blocks, thinking blocks, server tools, `mcp_servers`, cache/context/platform controls, and Responses `background`, `store`, `previous_response_id`, `conversation`, hosted tools, or non-text multimodal input.

## Consequences

- Positive: The existing proxy use case remains the single orchestration authority for auth, quota, routing, audit, retry, billing, and logs.
- Positive: Downstream protocol compatibility can be added without duplicating provider selection or request logging logic.
- Positive: Provider adaptation and downstream protocol conversion have clear boundaries, avoiding a broad "protocol conversion" bucket.
- Positive: A thin registry gives handlers a uniform path for Chat, Messages, and Responses while keeping the first version simple.
- Positive: `ConversionReport` makes lossy or normalized behavior observable without forcing a request-log schema migration immediately.
- Negative: Native upstream endpoint selection is deferred; for example, `/v1/responses` will initially be represented through canonical chat instead of preferring a native upstream Responses endpoint.
- Negative: Logs will initially show the canonical body, not the raw Anthropic or Responses client payload, which limits client-protocol debugging.
- Negative: Some protocol features will be rejected even when a future endpoint-aware planner could support them through a native upstream endpoint.
- Neutral: `protocol/` may depend on response envelope types such as `ProviderResponse`, `StreamEvent`, and `TokenUsage`, but it must not depend on `Channel`, `ProviderAdaptor`, `ProviderError`, `reqwest`, or `axum`.
- Neutral: If conversion complexity grows, the registry can later evolve from function dispatch to explicit codec traits and versioned directions.

## Alternatives

- Put all protocol compatibility inside provider adapters. Rejected because downstream client protocol conversion and upstream provider adaptation have different headers, error bodies, auth surfaces, SSE grammar, and observability needs.
- Merge provider adaptation and downstream protocol conversion into one protocol engine. Rejected because it would blur the boundary between client-facing compatibility and upstream HTTP/provider concerns.
- Introduce endpoint-aware route planning immediately, following the full `waliapi` route-plan model. Rejected for the first version because it would require endpoint capability modeling, native/conversion route groups, prepared attempts, richer failure classes, stream commit barriers, migrations, and a broader test matrix.
- Create a separate `ProtocolGatewayUsecase` parallel to `ProxyRequestUsecase`. Rejected because it would duplicate or wrap the existing auth, quota, routing, audit, retry, billing, and logging closed loop.
- Store raw downstream request bodies and conversion reports immediately. Rejected for the first version because existing log parsing and security audit logic are canonical-chat-shaped; adding raw body storage should be a separate observability decision.
- Fully model official OpenAI/Anthropic schemas as Rust structs. Rejected because first-version compatibility only needs a controlled subset; exhaustive schema modeling would add churn without improving the core architecture.

## Reversal Criteria

Revisit this decision when:

- `/v1/messages` or `/v1/responses` need native upstream endpoint preference for correctness rather than compatibility.
- Unsupported stateful Responses features such as `background`, `store`, `previous_response_id`, or `conversation` become required product behavior.
- Users need to debug raw downstream payloads, making canonical-only request logging insufficient.
- Conversion reports become important enough for UI display, audit export, or support workflows.
- Protocol count or conversion directions grow enough that function dispatch in `CodecRegistry` becomes hard to maintain.
- Provider adapters and downstream conversion modules develop repeated pure mapping logic that is stable enough to extract into shared protocol helpers.
