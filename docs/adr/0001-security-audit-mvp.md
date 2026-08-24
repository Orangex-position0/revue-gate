# Security Audit MVP Shape

- **Status**: accepted
- **Date**: 2026-08-18
- **Authors**: Orangex-position0 / Codex

## Context

revue-gate already records request-level facts in `request_logs`, including local API key, channel, model, usage, status, trace id, and request body. The v0.2.0 security audit feature needs to detect risky request content before upstream forwarding without turning the local gateway into a full compliance platform.

## Decision

We will implement security audit first as a request-log risk extension, not as a standalone `audit_events` subsystem. The first version scans request-side content after authentication and routing context is known, stores a request-level `RiskReport` on `request_logs`, and defaults to observe mode.

Specifically:

- Store `risk_level`, `audit_action`, and structured `audit_report_json` on `request_logs`.
- Shape `audit_report_json` around future `AuditFinding` rows so it can later migrate to an `audit_events` table.
- Scan user and tool messages, tool definitions, and non-basic top-level params by default.
- Skip system messages unless `scan_system_messages` is enabled.
- Represent the audit scope as flattened scan items with JSON Pointer paths, not as a copy of the full payload.
- Audit each logical request once after the first planned channel is known; retry attempts reuse the same `RiskReport`.
- Support `LogOnly`, `Warn`, and `Block` behavior in v0.2.0; keep `Redact` in the model but do not rewrite forwarded payloads yet.
- Return `403 Forbidden` with error type `security_policy_blocked` for blocked requests.
- Block only when `mode = Enforce`, `block_critical = true`, and a critical finding suggests block.
- Do not forward blocked requests upstream and do not accumulate provider usage for them, but still write a local request log.
- Add a thin `AuditDetector` trait and a `DefaultAuditEngine`; do not add a plugin system, hook dispatcher, CEL, or Rego in the MVP.
- Keep built-in detectors conservative: deterministic secrets, paths, Unicode obfuscation, prompt-injection phrases, tool risk, network literals, PII, and tracking patterns.
- Truncate scan input and finding output with explicit counters so logs stay bounded.
- Defer standalone `Rule Registry`, `Evidence Store`, and `Audit Log` modules to the next security-audit version.
- Add `GatewaySettings.audit`; new installs default to not storing full payloads, while existing installations should preserve current behavior or prompt the user.

## Consequences

- Positive: The MVP fits the existing proxy and log detail flow and can ship without a separate security center.
- Positive: The JSON shape keeps a migration path to independent audit events.
- Negative: Fine-grained filtering by rule, category, or evidence requires JSON querying or a later table split.
- Negative: Redaction is deferred, so the first version observes or blocks but does not safely transform risky prompts.
  - Note (2026-08-20): payload redaction of the *stored* `request_logs.request_body` (matching detector spans replaced, JSON structure preserved, decoupled from the audit master switch, upstream payload never rewritten) landed as the secondary-leak defense; redact-and-forward remains deferred.
- Neutral: Per-channel audit policy is deferred; one logical request produces one report even when forwarding retries across channels.
- Neutral: Response audit remains a future extension because streaming response scanning has different latency and truncation semantics.
- Neutral: Rule metadata, evidence retention, and independent audit-event querying are deliberate next-version concerns.

## Alternatives

- Create an `audit_events` table immediately. Rejected for MVP because it adds repository, join, UI, retention, and consistency work before the domain model has real usage pressure.
- Create standalone `Rule Registry` and `Evidence Store` modules immediately. Rejected for MVP because built-in deterministic rules can live in detector constants and redacted finding JSON until users need rule administration or independent evidence retention.
- Default to enforcement. Rejected because early deterministic detectors will have false positives, and a local gateway must preserve forwarding reliability by default.
- Return `451` for blocked requests. Rejected because mainstream gateways usually use `400` or custom codes, while `451` implies legal restriction; revue-gate's block is a local security policy decision, so `403` is clearer.
- Implement redaction before blocking. Rejected because JSON path rewrite, token changes, upstream/body divergence, and streaming semantics make redaction a larger feature than first-pass audit.

## Reversal Criteria

Revisit this decision when:

- Users need a security event center, JSONL export, independent retention, or cross-request rule analytics.
- The first detector set proves stable enough to safely default selected critical rules to enforcement.
- Response-side or streaming audit becomes part of the product surface.
