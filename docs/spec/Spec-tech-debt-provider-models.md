# Tech Debt And Provider Models Spec

> Status: ready-for-agent
> Source: `docs/Tech-debt.md` grilling session on SQLite cleanup, domain naming, provider model discovery, and DeepSeek support.
> Issue tracker: not configured in this workspace, so this spec is recorded in `docs/`.

## Problem Statement

revue-gate has several small but related backend debt items that make future provider work harder than it needs to be. SQLite repositories duplicate date parsing and repository error helpers, while timestamp formatting is not consistently fixed-width. `ChannelType` string conversion lives in infrastructure instead of the domain model. Local API Key generation is hidden in a usecase even though the key prefix and shape are domain rules. Provider response usage is named too broadly as `Usage`.

At the same time, provider model handling needs to evolve. Current provider adaptors expose static `default_models()` lists, which are useful as defaults but cannot reflect which Models are actually available to a user's upstream account. DeepSeek is also missing as a Provider even though its API is OpenAI-compatible. These changes should improve provider support without triggering a whole architecture rewrite.

## Solution

Implement the work in four independent slices:

1. Clean up backend domain and SQLite debt while preserving current behavior.
2. Fix the stale backend architecture document entry for the removed `utils` directory.
3. Add manual provider Model discovery while keeping static defaults as a fallback.
4. Add DeepSeek as an OpenAI-compatible Provider through shared OpenAI-compatible behavior, not by copying the existing OpenAI adaptor.

The existing `domain / usecases / infrastructure / interface` architecture stays in place. Gateway language should stay precise: use Provider, Upstream Channel, Model, Token Usage, Local API Key, and Channel Cooldown consistently. Do not add a `circuit` module as part of this work.

## User Stories

1. As a maintainer, I want SQLite timestamp formatting to be centralized, so that new SQLite code does not accidentally break lexicographic time comparisons.
2. As a maintainer, I want SQLite repositories to share RFC3339 parsing and repository error helpers, so that a small error handling change does not require three edits.
3. As a maintainer, I want old SQLite timestamp data to remain readable, so that a cleanup does not become a migration project.
4. As a maintainer, I want newly written SQLite UTC timestamps to use fixed 9-digit nanosecond precision, so that string range comparisons remain reliable.
5. As a maintainer, I want `ChannelType` string conversion to live in the domain model, so that infrastructure is not the authority for domain values.
6. As a maintainer, I want unknown persisted channel type strings to become bad-row repository errors, so that corrupt data is reported at the persistence boundary.
7. As a maintainer, I want Local API Key generation to live with the API key domain model, so that the `sk-revue-<16 hex>` rule has one owner.
8. As a maintainer, I want entity IDs and trace IDs to stay at their current boundaries, so that unrelated identity concepts are not forced into a global utility drawer.
9. As a maintainer, I want `Usage` to become `TokenUsage`, so that token accounting is not confused with quota usage, billing usage, or channel usage.
10. As a maintainer, I want architecture docs to stop mentioning a deleted `utils` directory, so that new contributors are not pointed at a nonexistent module.
11. As a user configuring an Upstream Channel, I want a static default Model list to appear immediately, so that channel setup works offline and remains fast.
12. As a user configuring an Upstream Channel, I want to manually fetch the provider-reported Model list, so that I can choose from Models my account can actually access.
13. As a user configuring an Upstream Channel, I want failed Model discovery to keep my current defaults, so that a provider API error does not erase useful configuration.
14. As a user configuring an OpenAI-compatible Provider, I want Model discovery to use the provider's `/v1/models` endpoint when supported, so that compatible providers behave consistently.
15. As a user configuring a Provider that cannot report Models, I want the UI to show that discovery is unsupported, so that I understand why only default Models are available.
16. As a user, I want DeepSeek to be available as a Provider, so that I can route local authenticated requests to DeepSeek through revue-gate.
17. As a maintainer, I want DeepSeek to reuse OpenAI-compatible request and response mapping, so that future compatible providers do not duplicate a full adaptor.
18. As a maintainer, I want the provider evolution to avoid a large provider framework rewrite, so that the code stays small and reviewable.
19. As a maintainer, I want Channel Cooldown to remain a future optimization, so that no half-built circuit breaker appears before failure metrics exist.
20. As a maintainer, I want `docs/Tech-debt.md` to be updated after completed slices, so that resolved debt does not remain as stale work.

## Implementation Decisions

- Add SQLite-local shared helpers inside the SQLite infrastructure module: `fmt_utc`, `parse_utc`, `db_err`, and `bad_row`.
- Keep these SQLite helpers `pub(crate)` and scoped to SQLite repository code. Do not introduce a global `utils` module for them.
- `fmt_utc` writes UTC timestamps with fixed 9-digit nanosecond precision.
- `parse_utc` continues to accept existing RFC3339 strings, including old variable-precision values.
- Do not migrate or rewrite historical `channels` or `api_keys` timestamps.
- Replace per-repository copies of SQLite parsing and error helpers with the shared SQLite helpers.
- Move `ChannelType` string authority into the domain model with `Display` and `FromStr`.
- Introduce a lightweight channel type parse error in the domain model; do not reuse repository errors in domain code.
- Map channel type parse failures to bad-row repository errors at the SQLite boundary.
- Move Local API Key value generation into the API key domain model.
- Keep database primary key generation in the usecases that create entities.
- Keep trace ID generation at the HTTP and observability boundary.
- Preserve the existing test sample strategy that avoids UUID v7 same-millisecond collisions.
- Rename provider token accounting from `Usage` to `TokenUsage`.
- Apply the rename through provider adaptor contracts and response mapping.
- Remove the stale `utils.rs` / `utils/` directory entry from the backend architecture document.
- Keep `default_models()` on provider adaptors as a static recommendation list and offline fallback.
- Add a provider adaptor capability for fetching provider-reported Models.
- Model fetching must be user-triggered from the channel configuration UI. It should not automatically block channel form loading.
- Model fetching failures must surface a clear error while preserving the current/default list.
- OpenAI-compatible Providers should share a default Model discovery implementation based on `GET /v1/models`.
- Providers that do not support remote Model discovery should explicitly report unsupported discovery instead of pretending an empty remote list is authoritative.
- Add DeepSeek as an OpenAI-compatible Provider configuration with its own provider identity, base URL, and default Models.
- Share OpenAI-compatible request and response mapping between OpenAI and DeepSeek.
- Do not copy the entire OpenAI adaptor to create DeepSeek.
- Do not add a `circuit` module in this spec.
- Do not perform an overall architecture rewrite in this spec.

## Testing Decisions

- The highest useful seam for backend cleanup is existing repository and usecase tests. Prefer asserting behavior through repository operations rather than unit-testing helper implementation details.
- SQLite tests should prove fixed-width timestamps are written for new rows and that old variable-precision RFC3339 strings remain readable where practical.
- Channel type tests should cover display strings, parsing known values, and rejecting unknown strings.
- API key tests should assert the public key shape and prefix through the domain-facing generation function.
- Token usage rename should be verified by the Rust compiler and existing provider response mapping tests.
- Architecture document cleanup needs no automated test beyond review.
- Model discovery should be tested at the provider adaptor seam with mocked provider responses for success, malformed responses, provider errors, and unsupported providers.
- Usecase or command tests should cover preserving fallback/default Models when fetch fails.
- Frontend tests, if the project already has them for channel forms, should cover the manual fetch button loading, success, failure, and unsupported states. If no frontend test seam exists, verify with the smallest available build/typecheck.
- DeepSeek should be tested through the shared OpenAI-compatible adaptor seam to confirm provider identity, base URL selection, default Models, request mapping, and response mapping.
- The full verification pass should include Rust tests and the project's existing frontend typecheck/build command if provider UI changes are included.

## Out of Scope

- Migrating historical SQLite timestamp values.
- Adding a global `utils` module or `id_generator` module.
- Moving all UUID primary key generation into a central service.
- Moving trace ID generation out of the HTTP/observability boundary.
- Replacing `default_models()` with remote-only discovery.
- Automatically fetching Models whenever the channel form loads.
- Adding Channel Cooldown.
- Adding a `circuit` or circuit breaker module.
- Reworking quota, routing, retry, fallback, or security audit behavior.
- Rewriting the project architecture or replacing the current layering.
- Publishing to an external issue tracker from this workspace.

## Further Notes

- `Channel Cooldown` remains future work and depends on failure metrics. It should not be represented as a circuit breaker until the project has explicit state semantics such as open, closed, and half-open.
- This spec intentionally groups several small cleanup items with provider model evolution because they all reduce ambiguity around gateway domain language before adding more Providers.
- After each implemented slice, update `docs/Tech-debt.md` to remove or narrow completed entries.
