# revue-gate Context

revue-gate is a local AI gateway that routes authenticated client requests to upstream model providers while keeping operational and security evidence on the user's machine.

## Language

**Local API Key**:
A gateway-issued key used by a client to authenticate with revue-gate. It is the governance subject for quota, logs, and security audit.
_Avoid_: Provider key, upstream key

**Upstream Channel**:
A configured provider endpoint that revue-gate can forward a model request to after local authorization and routing.
_Avoid_: Route, backend, provider account

**Provider**:
An upstream model service family, such as OpenAI-compatible, Anthropic, Gemini, or DeepSeek. A provider is not the same as a configured upstream channel.
_Avoid_: Channel, model

**Model**:
The upstream model identifier selected for a logical request, either from revue-gate's default catalog or from a provider-reported model list.
_Avoid_: Provider, channel

**Token Usage**:
The prompt, completion, and total token counts reported for a logical request or upstream attempt.
_Avoid_: Usage, quota usage, billing usage

**Channel Cooldown**:
A temporary routing exclusion for an upstream channel after repeated retryable failures. It is lighter than a full circuit breaker and does not imply a Closed/Open/HalfOpen state machine.
_Avoid_: Circuit breaker, circuit

**Request Log**:
The request-level record that explains what revue-gate did for one client request, including routing, status, usage, trace, and audit summary.
_Avoid_: Access log, audit event

**Logical Request**:
One client request as seen by revue-gate, regardless of how many upstream retry attempts it produces.
_Avoid_: Attempt, upstream call

**Security Audit**:
The request-time risk assessment performed by revue-gate before forwarding content to an upstream channel.
_Avoid_: Compliance audit, log review

**Audit Finding**:
A single risk hit produced by a detector, with rule identity, category, level, action suggestion, location, and redacted evidence.
_Avoid_: Alert, violation

**Risk Report**:
The request-level aggregation of audit findings into one risk level, score, and final security action.
_Avoid_: Finding list, scan result

**Detector**:
A built-in scanner that inspects an audit scope and produces audit findings without deciding the final request outcome.
_Avoid_: Rule, policy, plugin

**Audit Policy**:
The resolved security audit settings used by the data plane to decide scan scope, evidence retention, and whether critical findings may block a request.
_Avoid_: Settings, rule registry

**Rule Registry**:
The catalog of audit rule metadata, including rule identity, category, default level, default action, scope, version, and enablement.
_Avoid_: Detector, policy

**Audit Scope**:
The selected request or response fields that detectors are allowed to inspect, which may be narrower than the full upstream payload.
_Avoid_: Payload, request body

**Audit Scope Item**:
One flattened scan target inside an audit scope, carrying a JSON Pointer path, a scope kind, and extracted text.
_Avoid_: Finding, JSON field

**Evidence Store**:
The dedicated store for audit finding evidence when evidence needs independent retention, export, review, or deduplication.
_Avoid_: Request log, full payload store

**Audit Event**:
An independently queryable security audit record derived from one finding or security-relevant change.
_Avoid_: Request log, risk report

**Payload Redaction**:
The rewriting of a stored Request Log's request body so matching detector spans are replaced with a redaction placeholder, keeping the JSON structure and all non-matching content intact. It applies to the local stored copy only; the forwarded upstream payload is never rewritten. Independent of the audit master switch: any stored payload is redacted regardless of whether audit is enabled.
_Avoid_: Sanitization, masking, redact-and-forward

**Security Action**:
The final handling decision for an audited request: allow, log only, warn, redact, confirm, or block.
_Avoid_: Status, result
