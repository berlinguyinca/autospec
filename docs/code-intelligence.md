# Code Intelligence Gateway

AutoSpec gives Pi agents IDE-grade semantic navigation — definitions,
references, implementations, callers, diagnostics and impact analysis — from
self-hosted language servers. Source and indexes stay on the AutoSpec execution
host; InferWeave only ever receives the context selected for the model.

No paid external service is required.

## Why a gateway

Agents never get raw LSP JSON-RPC. They call a small AutoSpec-owned surface,
and a backend adapter translates it. That buys three things:

1. **Isolation.** Every query names a workspace, and a workspace resolves to
   exactly one worktree root. Diagnostics, document state and cache entries can
   never cross worktrees.
2. **Provenance.** Every result says which workspace, repository, revision and
   backend produced it, and how far to trust it. A textual guess is never read
   as a semantic fact.
3. **Replaceability.** agent-lsp is the v1 backend, but it is pinned behind
   `CodeIntelBackend`. Microsoft multilspy and lsproxy are sibling adapters,
   not rewrites.

```text
Pi agents
   |
AutoSpec Agent Tools
   |
Code Intelligence Gateway  ----  Context Builder / RAG
   |
agent-lsp / ast-grep / ripgrep
   |
rust-analyzer / pyright / jdtls / gopls / tsserver / clangd / metals / ...
```

## The API

Ten read-only operations, all namespaced `code.`:

| Operation | Returns |
|---|---|
| `code.find_symbol` | symbols matching a name |
| `code.definition` | defining symbol(s) |
| `code.references` | every reference, flagged read/write/definition |
| `code.implementations` | implementations of a trait/interface |
| `code.hover` | type and documentation at a position |
| `code.callers` | incoming call hierarchy |
| `code.callees` | outgoing call hierarchy |
| `code.type_hierarchy` | super/sub types |
| `code.diagnostics` | current diagnostics for a workspace |
| `code.impact` | the aggregate blast radius (see below) |

`code.impact` is the primary high-level operation. It returns definitions,
callers, callees, references, implementations, exports, related tests,
dependent modules, diagnostics and the union of affected files — the evidence a
plan's Impact Set is checked against.

Every result carries provenance:

```yaml
workspace: issue-421
repository: inferweave-gateway
revision: abc123
backend: agent-lsp
source: lsp
confidence: semantic
```

Fallback results report `source: ast-grep` / `ripgrep` and
`confidence: structural` / `textual`, and set `degraded: true`.

## Worktree isolation

Each active worktree gets one logical semantic workspace:

```text
main/                           -> repo-main
.autospec/worktrees/issue-421/  -> issue-421
.autospec/worktrees/issue-422/  -> issue-422
```

Enforced invariants:

- Every query carries a workspace ID.
- A workspace resolves to exactly one root; two workspaces may not share one.
- A path that escapes the worktree (absolute, or `..` past the root) is refused.
- Cache keys are `workspace:revision:operation:request` — a result can never be
  reused across worktrees or across revisions of the same worktree.
- Deleting a worktree drops its semantic state entirely.

Lifecycle: create worktree → detect languages → provision servers → warm →
pre-change impact → plan/implement/test/review → final diagnostics → merge →
expire workspace → remove worktree. Warm workspaces expire after
`workspace.idle_ttl_minutes` (default 30).

## Diagnostics and the diagnostic delta

Diagnostics are captured before implementation, after meaningful edit batches,
around tests/builds, and before review completion. The gateway compares
snapshots and reports a delta.

Diagnostic identity is `file + severity + code + message` — deliberately **not**
line number. Editing a file shifts every diagnostic below the edit; keying on
line would report those as new errors.

Baseline errors stay visible but never fail a task on their own. Only errors
the change introduced are gated, under `workflow.block_new_errors`.

## Mandatory semantic gates

| Role | Gate | Blocks when |
|---|---|---|
| Planner | `pre-change-impact` | no analysis, an empty analysis, or a plan with no declared Impact Set |
| Implementer | `semantic-change` | no post-change diagnostic delta was captured, or the delta contains new errors |
| Reviewer | `independent-analysis` | the reviewer ran no analysis of its own, or reused the implementer's |

A reviewer analysis counts as independent only if it differs from the
implementer's in provenance or in the sets it produced. Reusing the
implementer's report is the specific failure this gate exists to catch.

Gate outcomes record `degraded: true` when the analysis behind them fell back
below the semantic tier. A task may continue in degraded mode, but its record
shows that it did.

## Fallback

```text
agent-lsp / LSP  --X-->  ast-grep / tree-sitter  --X-->  ripgrep
```

Degradation is automatic unless the caller requires semantic certainty. Two
kinds of failure never degrade:

- **Config and gate failures** — operator errors, not backend flakiness.
- **Type-dependent operations** — `code.hover`, `code.type_hierarchy` and
  `code.diagnostics` need a type checker. No structural or textual matcher can
  synthesize them, so they fail closed rather than guessing.

## Languages

| Language | Default server | Detected by |
|---|---|---|
| Rust | `rust-analyzer` | `Cargo.toml`, `*.rs` |
| Python | `pyright-langserver` | `pyproject.toml`, `setup.py`, `setup.cfg`, `requirements.txt`, `*.py` |
| TypeScript | `typescript-language-server` | `tsconfig.json`, `*.ts`/`*.tsx` |
| JavaScript | `typescript-language-server` | `package.json`, `jsconfig.json`, `*.js`/`*.jsx` |
| Java | `jdtls` | `pom.xml`, `build.gradle[.kts]`, `*.java` |
| Go | `gopls` | `go.mod`, `*.go` |
| C | `clangd` | `compile_commands.json`, `*.c`/`*.h` |
| C++ | `clangd` | `CMakeLists.txt`, `*.cpp`/`*.hpp` |
| C# | `omnisharp` | `*.csproj`, `*.sln`, `*.cs` |
| Kotlin | `kotlin-language-server` | `build.gradle.kts`, `*.kt`/`*.kts` |
| Scala | `metals` | `build.sbt`, `*.scala` |

A Maven/Scala project is detected by its `.scala` sources: `pom.xml` alone says
Java, not Scala.

Every default is overridable under `languages.overrides`.

## Configuration

`.autospec/code-intelligence.yaml`. Unknown keys are **rejected**, not ignored —
a typo in a gate or security key must fail loudly rather than silently turning
that control off.

```yaml
version: 1
enabled: true

backend:
  type: agent-lsp
  mode: local          # local | container | http
  binary: agent-lsp

workspace:
  isolation: worktree  # worktree | repository
  idle_ttl_minutes: 30
  warm_cache: true

languages:
  auto_detect: true
  overrides:
    python: { server: pyright-langserver }

fallback:
  structural: ast-grep
  textual: rg

workflow:
  require_pre_change_impact: true
  require_post_change_diagnostics: true
  reviewer_independent_analysis: true
  block_new_errors: true

context:
  prefer_semantic: true
  include_related_tests: true
  include_rag: true
  include_git_history: targeted   # targeted | none | full

security:
  allow_public_bind: false
  trust_project_build_scripts: false
```

An absent file uses these defaults. A malformed file is an error.

## Security

- The backend runs at the same or lower privilege than the agent.
- Only the relevant worktree is mounted; no unrelated host mounts.
- `local` and `container` modes open no socket. `http` mode is the only one
  that requires authentication, and `http` + `allow_public_bind: true` is
  reported as a blocking failure by `autospec doctor code-intel`.
- Java, Kotlin, Scala and Python servers may execute project build code during
  startup. That stays gated behind `security.trust_project_build_scripts`,
  which defaults to `false`.
- Backend and server versions are recorded in every report.

## Operating

```bash
autospec doctor code-intel          # human-readable
autospec doctor code-intel --json   # machine-readable
```

The report covers backend presence and version, detected languages and whether
each server is installed, fallback tool availability, the security posture, and
a definition/reference smoke probe. A missing backend or a publicly-bound HTTP
backend blocks; a missing language server warns and notes that those queries
will fall back.

## Observability

Per workspace and language: operation counts, p50/p95/p99 latency, cache-hit
rate, fallback rate and failure rate. Percentiles are nearest-rank, so every
reported number is an observed latency rather than an interpolation.

## Upstream

- agent-lsp: <https://github.com/blackwell-systems/agent-lsp>
- Microsoft multilspy: <https://github.com/microsoft/multilspy>
- lsproxy: <https://github.com/agentic-labs/lsproxy>

agent-lsp tool names are an upstream contract, not a standard. They live in
exactly one table — `TOOL_NAMES` in
`crates/autospec-core/src/code_intel/backend/agent_lsp.rs` — alongside the
pinned version. Revalidate that table when upgrading the dependency.
