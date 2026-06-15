# autospec-secaudit — Security / IP / Leak Guard Skill

**Date:** 2026-06-14
**Status:** Design (approved sections 1–3, naming confirmed)
**Author:** berlinguyinca

## Problem

autospec generates and auto-merges code autonomously. Today the only security
touchpoint is a single `SECURITY` directive in the Phase 4 implementer contract
and a generic codex peer-review pass that mentions "security" among other
concerns. There is no dedicated, deterministic gate that prevents the autonomous
loop from merging code that:

- contains vulnerabilities (injection, unsafe deserialization, path traversal,
  unsafe `eval`/shell, auth/authz flaws, **backdoors / bad patterns**),
- leaks **secrets / credentials** (hardcoded keys, tokens, private keys),
- violates **IP / copyright / license** (verbatim copied code, copyleft
  contamination, license-incompatible dependencies),
- exposes **PII / sensitive data** (PII in logs, exfiltration to external
  services),
- is vulnerable to **SQL injection** or **prompt injection** (untrusted input
  reaching LLM sinks).

## Goal

A dedicated security guard with **two entry points sharing one detection
engine**:

1. **Phase 4 per-PR gate** — blocking, runs on the PR diff before merge inside
   `autospec-run`, with an auto-fix + re-scan remediation loop.
2. **Standalone `/autospec-secaudit` skill** — repo-wide sweep an operator
   invokes, and which auto-fires after each `autospec-run` batch (like
   `autospec-review`) to catch cross-PR integration leaks.

Detection = **deterministic scanners first, LLM triage second.** Scanners are
**auto-installed on first run** (best-effort). Findings normalize to the existing
gap JSON schema so they reuse dedupe + filing infrastructure.

## Chosen approach

**One standalone skill; Phase 4 calls the same shared script.** One detection
engine, two entry points — no logic duplication, single lock-step surface.

Rejected:
- *Two separate implementations* (inline Phase 4 + independent skill) — duplicates
  orchestration, drifts out of lock-step.
- *Pure LLM rubric* — user requires deterministic scanners + auto-install; LLM
  alone misses entropy-based secrets and CVEs.

## Components & layout

New trio skill `skills/autospec-secaudit/` + shared engine scripts:

| File | Role |
|------|------|
| `skills/autospec-shared/scripts/security-scan.sh` | Run deterministic scanners over a diff (`--diff <base>`) or full tree (`--tree`); normalize every finding into gap JSON `{gap_id,dimension,severity,file,line,title,body,dedupe_key}`; emit one stream. |
| `skills/autospec-shared/scripts/security-remediation-loop.sh` | Wrap scan → LLM-triage → auto-fix → re-scan, capped at `AUTOSPEC_SEC_MAX_ROUNDS` (default 3). Modeled on `gap-remediation-loop.sh`. |
| `skills/autospec-shared/scripts/ensure-tool.sh` (extend) | Add `gitleaks`, `semgrep`, `trivy`, `license-checker` (npm) to the baked-in table so first-run auto-installs them, best-effort, honoring existing `AUTOSPEC_SKIP_ENSURE_TOOL_*` opt-outs. |
| `skills/autospec-secaudit/SKILL.md` + `codex/prompt.md` + `opencode/agent.md` | Operator-facing standalone sweep; auto-fires post-batch unless `~/.autospec/no-secaudit.flag` exists. |
| `skills/autospec-secaudit/install.sh` / `uninstall.sh` / `README.md` / `tests/` | Match sibling-skill structure. |

### Scanner → concern mapping

| Concern class | Primary scanner | LLM augmentation |
|---|---|---|
| Secrets / credentials | gitleaks | confirm context, flag rotation |
| Vulns, SQLi, cmd/eval injection, path traversal, deserialization, backdoors/bad patterns | semgrep (security + bad-pattern rulesets) | triage false positives, add patterns rulesets miss |
| Prompt injection (untrusted input → LLM sink) | semgrep custom rules | primary detection (LLM data-flow reasoning) |
| Dependency CVEs + IaC | trivy | summarize, assess fix availability |
| IP / copyright / license | license-checker + copyleft/verbatim heuristic | judge copyleft contamination, attribution |
| PII / data leaks | — | primary detection (PII-in-logs, exfiltration) |

## Data flow

### Standalone `/autospec-secaudit`
1. `ensure-tool.sh` each scanner (best-effort auto-install; loud WARN + LLM-only
   fallback for any concern whose scanner failed to install — e.g. locked-down
   env).
2. `security-scan.sh --tree` (or `--diff <base>` for a branch) → normalized
   gap-JSON stream.
3. LLM triage pass (Tier-A model): confirm real vs false-positive, add
   PII/prompt-injection findings, assign `severity` (`must-fix` / `nice-to-have`).
4. Write report to `.autospec/secaudit.md`; survivors above threshold filed as
   `auto-implement,security,priority:high` issues via the existing gap-filing
   path so they re-enter autospec. `dedupe_key` prevents re-filing across batches.

### Phase 4 per-PR gate (blocking)
1. After implement, before merge: `security-remediation-loop.sh --diff main...HEAD`
   runs the **same** scan on the PR diff only.
2. On `must-fix` findings → feed back to the implementer as `SECURITY` directives
   (extends the existing implementer-contract row), implementer fixes, **re-scan**.
   Loop up to `AUTOSPEC_SEC_MAX_ROUNDS` (default 3).
3. **Secrets** get special handling: auto-redact/remove the secret AND flag it
   for **rotation** in the PR body — a committed secret is compromised even after
   removal, so a code fix is not full remediation.
4. If a `must-fix` survives all rounds → **block merge**, post findings to the PR,
   surface to operator. `nice-to-have` never blocks.

### Concern → enforcement defaults

| Concern | Phase 4 gate | Auto-fix |
|---|---|---|
| Secrets / credentials | block | redact + flag rotation |
| Code vulns / SQLi / cmd-injection / backdoors | block | yes, re-scan |
| Prompt injection | block | yes, re-scan |
| PII / data leaks | block | yes, re-scan |
| IP / copyright / license | block | advisory fix (human confirms license calls) |
| Dependency CVEs (trivy) | advisory (block only on critical **with** fix available) | bump if trivial |

## Error handling & graceful degradation

- Every scanner runs best-effort via `ensure-tool.sh` (exit 0 always). Missing /
  failed scanner → loud `WARN` line in the report naming the gap, then LLM-only
  fallback for that concern. **Never silently skip** (no-silent-caps rule).
- **Fail-closed:** if the gate engine cannot run at all, it exits non-zero so
  Phase 4 blocks rather than merging blind. (Contrast: a missing single scanner
  degrades; a broken engine blocks.)
- `AUTOSPEC_SEC_MAX_ROUNDS` (default 3) caps the fix/re-scan loop; on exhaustion
  → block + report.
- Findings reuse `gap-json-lib.sh` for dedupe + filing; `dedupe_key` stops
  re-filing the same finding each batch.

## Testing (bats, matching `skills/autospec-shared/tests/`)

- `security-scan.sh`: fixture diffs with a planted real-world AWS key, a SQLi
  sink, `eval(user_input)`, a GPL license header, and a PII-in-log line → assert
  each maps to the correct `dimension` + `severity`. **Fixtures use raw
  real-world strings, NOT the SUT's own derivation expression** (self-consistent
  fixtures mask bugs).
- Scanner-absent path: stub `command -v` to fail → assert WARN + LLM-fallback,
  exit 0.
- Remediation loop: mock scan returning must-fix twice then clean → assert ≤3
  rounds; assert block on surviving must-fix; assert secret-redaction path.
- One negative-path pair per concern class (tracker #420 discipline).

## Lock-step & validation (mandatory)

- All three trio files (`SKILL.md`, `codex/prompt.md`, `opencode/agent.md`) carry
  identical prose blocks: **Self-update mode**, **Required-capabilities harness-
  adapter table**, **Model-tier row**. Watch the `codex/prompt.md` leading-blank-
  line gotcha.
- `validate.sh` `check_lockstep()` must guard the new trio; add named-content
  checks for the new skill.
- First decomposed issue for this skill MUST include the structural sections
  (Self-update + Model-tier + adapter row). Decomposer should NOT apply the
  needs-autospec-template label.

## Configuration / env

| Var / flag | Default | Effect |
|---|---|---|
| `AUTOSPEC_SEC_MAX_ROUNDS` | 3 | Fix/re-scan loop cap |
| `~/.autospec/no-secaudit.flag` | absent | Disables post-batch auto-fire |
| `AUTOSPEC_SKIP_ENSURE_TOOL_GITLEAKS` etc. | unset | Per-scanner auto-install opt-out (existing mechanism) |
| `.autospec/secaudit.md` | — | Standalone sweep report output |

## Out of scope (YAGNI)

- Custom secret-scanning rules beyond gitleaks defaults (revisit if FP/FN rate
  warrants).
- Runtime/DAST scanning — static + dependency only.
- SBOM generation (trivy can, but not a goal here).
- Replacing the existing codex peer-review pass (complementary, not a
  replacement).

## Open questions

None blocking. License-call ambiguity (copyleft contamination) is intentionally
left as advisory-fix requiring human confirmation rather than auto-edit.
