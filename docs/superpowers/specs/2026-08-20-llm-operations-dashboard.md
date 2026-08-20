# LLM operations dashboard design

## Outcome

The existing `/#overview` route becomes a fleet operations view that answers, without drill-down: which models and quantized artifacts are loaded, which servers and GPUs are carrying work, how many requests and authenticated principals are active, what is queued, and what throughput is live versus historically measured.

The model, server, usage, and health pages remain drill-downs. The public page keeps aggregate operational visibility. A member additionally sees their own active credentials, while an administrator may see every active identity.

## Evidence and constraints

- `web/index.html` already joins `/api/stats`, `/api/servers`, and `/api/usage`, but only on separate tabs.
- `server.models` is a capability list. It is not a reliable statement that each model is resident.
- Local llama.cpp model records contain `status.value`; the gateway currently discards it. Generic OpenAI-compatible servers may omit it.
- `keystore.throughput()` already computes prefill and prediction rates over seven days; the fleet API currently drops prediction rate and does not name the window.
- `INFLIGHT` counts requests per server, but no registry connects current work to a client.
- Local decode rate is live when metrics are readable. Remote rates are completed-request history, not live GPU counters.
- `config/model-artifacts.yaml` already records the artifact id, role, architecture, size, quantization method/variant/Dynamic generation, verified context, and declared GPU layout. It is the authoritative source for local model metadata.

## Approaches considered

### 1. Compose existing payloads only

Fastest, but it would infer residency from capability and call historical throughput current. Rejected because the resulting dashboard would be visually useful and operationally wrong.

### 2. Add one monolithic `/api/operations` snapshot

It could give the browser one payload, but it would couple the local collector, fleet registry, usage store, and activity state into a second aggregation authority. Rejected because the existing endpoints have distinct privacy and refresh contracts.

### 3. Extend the narrow source contracts and compose them in the browser

Selected. `/api/servers` gains nullable residency and measured decode fields; a focused `/api/activity` endpoint reports current gateway work with public/private projections; the browser joins those with `/api/stats`. Each source stays independently available and carries its own semantics.

## Backend contracts

### Model state

Every server row keeps `models` as the available capability list and adds:

- `loaded_models: list[str] | null`
- `loaded_state_source: "router-status" | "unsupported" | "stale"`
- `loaded_models_observed_at: number | null`

The gateway reads `status.value == "loaded"` from llama.cpp model entries. If no model entry exposes a residency status, it reports `null` and `unsupported`. A failed refresh may retain the last list only as `stale`. Public projection explicitly allow-lists these fields. Server rows also carry source-specific `models_observed_at`, `cards_observed_at`, and throughput refresh time; control-channel `last_seen` is not reused as model freshness.

### Model and deployment metadata

The installed dashboard reads the shipped artifact manifest and joins each preset to an artifact by the preset's model-file basename. The public model descriptor contains only an allow-listed profile:

```json
{
  "role": "primary",
  "architecture": "Qwen3_5ForConditionalGeneration",
  "weight_size_bytes": 16464440224,
  "verified_context_tokens": 99710,
  "quantization": {"method": "gguf", "variant": "UD-Q4_K_M", "dynamic": "v3.0"}
}
```

No repository path, revision, runtime argv, secret-file location, or raw `status` object is published. A remote OpenAI-compatible server may advertise the same allow-listed `quantization` object; otherwise its quantization is `null` with source `unsupported`. Quantization is deployment-specific because two servers may expose the same model id with different artifacts.

`GET /v1/models` remains OpenAI-compatible and adds optional extension fields for `kind`, `context`, `slots`, `profile`, and `deployments`. Each deployment names its public server id, pinned route, loaded state, residency provenance, and safe quantization metadata. `/api/servers` remains the source for server-level GPU cards.

The selection rule is deliberately two-dimensional:

- `model` selects weights, quantization, capabilities, and context profile;
- base URL `/u/<server>/v1` selects a deployment and therefore its server/GPU runtime;
- plain `/v1` lets the scheduler select among eligible deployments by measured load, throughput, and warm affinity.

A physical GPU is separately requestable only when its runtime is registered as a distinct server id. The gateway does not claim it can select one card inside a multi-GPU process.

### Throughput

Each server row exposes a `throughput_baseline` object with `prefill_tokens_per_second`, `predict_tokens_per_second`, `mean_model_service_seconds`, `samples`, `window_seconds: 604800`, and `refreshed_at`. The UI calls these measured baselines. Only `/api/stats.tokens_per_second` is labeled live decode.

### Active requests and principals

A small, lock-protected registry records a request after authentication and routing, marks it waiting before admission and running afterward, and removes it in an outer `finally` that covers every refusal and relay outcome. It tracks an opaque request token, `(sub, key_id)`, optional key label, requested model, upstream, phase, monotonic start time, and wall-clock observation time. It never records prompts, body fragments, addresses, or user agents.

The registry detail cap uses paired overflow tokens rather than evicting live rows: aggregate totals remain exact even if individual detail is capped.

`GET /api/activity` returns:

- public: counts by phase/server/model, with no identifiers;
- authenticated non-admin: the same counts plus only that subject's grouped credential rows;
- authenticated admin: the same counts plus grouped rows for every active subject and credential.

“Active” means a request currently waiting or running through this gateway. One credential may be shared by several processes and one person may own several credentials, so the UI says requests, principals, and credentials rather than claiming to count client processes. Callers that reach a server directly are not visible.

## Frontend design

The sidebar status card stops naming one local Qwen model. It shows fleet-wide loaded-model count, active request count, and GPU count.

The Operations page renders, in order:

1. Fleet KPIs: online servers, loaded models, active principals/requests, queued requests, live local decode, and GPUs doing work.
2. Loaded models: model id, host badges, residency source, and active request count. Available-but-not-known-loaded models are summarized separately.
3. Server load: state, in-flight work, held KV requests, measured prefill/decode rates, mean request duration, and sample count.
4. GPUs: the existing per-card cards, grouped by server.
5. Active work: public counts, a member's own credential rows, or administrator-wide identity rows.
6. Local queue and cumulative local counters: the existing cards, relabeled as local.

The Models page adds quantization, artifact role, verified context, and deployment routes. A new deep-linkable `/#guide` page renders the service documentation: a routing-flow diagram, the model-versus-deployment decision rule, balanced and pinned examples, privacy/telemetry boundaries, and a live service-catalog table populated from the same APIs as discovery.

There are no charts in this change: the current APIs do not carry time-series data, and a single bar would imply history that does not exist.

## Failure and privacy behavior

- A failed activity request does not blank model, server, or GPU telemetry.
- A failed fleet request does not stop the local stats poll.
- Unknown residency is rendered explicitly and never treated as unloaded.
- Public activity payloads contain neither subject ids, key ids, labels, email addresses, nor display names; non-admin payloads contain no other subject's detail.
- All new public fields are allow-listed; future backend fields fail closed by omission.
- The browser continues to drain every response body through `req()` and uses a single-flight poll so slow responses cannot overlap or overwrite newer state.

## Acceptance criteria

1. `/#overview` is labeled Operations and shows loaded models across every online server that reports residency.
2. A generic server without residency metadata renders “loaded state unknown”; its capability list remains available.
3. `/api/servers` exposes prediction rate in a baseline with `window_seconds=604800`, and the UI labels it measured rather than live.
4. While two requests from different subjects wait or run, `/api/activity` reports two active principals and two requests; after completion it reports zero.
5. Anonymous `/api/activity` contains counts but no identity or key fields; members see only their own detail and administrators see all detail.
6. The sidebar never presents the local resident model as though it were the fleet summary.
7. GPU cards include every reported card and retain their server label.
8. Queue, activity, and throughput unknowns render as em dashes or explicit unknown states, never fabricated zeroes.
9. Existing Models, Servers, Usage, and Health routes continue to work.
10. `/v1/models` advertises each known local artifact's quantization/profile and per-server deployment routes without exposing paths, revisions, argv, or secret metadata.
11. `/#guide` renders the routing branch from client to model eligibility to balanced or pinned deployment, with copyable examples for both paths.
12. A server/GPU can be selected through its advertised `/u/<server>/v1` route; the guide states that an individual card needs its own registered runtime/server id.
13. The joint Python suites, structural suite, Go agent suite, and relevant repository validation pass, with any pre-existing baseline failure reported separately.

## Deferred work

- Historical GPU graphs, request-rate charts, latency percentiles, TTFT, and error-rate series.
- Per-process GPU attribution.
- Observing traffic that bypasses this gateway.
- An atomic multi-source snapshot or a new telemetry database.
- Composite `model@server` aliases or request-body rewriting; deployment selection remains an explicit base-URL choice.
