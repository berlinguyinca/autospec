# autospec-fab vision stage — ship a real vision CLI consumer (design)

Status: proposed
Owner: berlinguyinca
Date: 2026-06-22
Slug: autospec-fab-vision-cli
Tracking issue: #1271 (area:fab)

## Problem

`skills/autospec-fab/scripts/stage_vision.py` is a well-formed **advisory** vision stage. It resolves a vision command from `$AUTOSPEC_FAB_VISION_CMD` (else PATH `fab-vision`), runs a JUDGE pass over the render contact sheet, runs an adversarial `--verify` pass, and records only confirmed observations into the non-blocking `vision_findings[]`. It degrades honestly: no command resolved → `status: "skip"`.

But **no real vision CLI ships**. `$AUTOSPEC_FAB_VISION_CMD` is unset and no `fab-vision` is on PATH in any real deployment, so `resolve_vision_cmd()` returns `None` and **every fab run SKIPS vision**. The stage has a defined API + a full test suite (`tests/test_stage_vision.py` exercises it through a PATH/env shim) but **zero real consumers** — exactly the ROI gap memory flags ("every new component needs a named consumer that benefits today"). The 2026-06-20 whole-project review surfaced this.

The contract is already proven plumbed end-to-end: `stage_render.py` writes `<render-dir>/<slug>/contact-sheet.html` (an HTML grid of per-view `<img>` tags), and `stl-release-gate.py` threads that sheet as the vision stage's `--in` (the render→vision handoff, #1266). The only missing piece is a real, runnable command that satisfies the existing `$AUTOSPEC_FAB_VISION_CMD` contract.

## Goal

Ship a **real, default, runnable vision CLI consumer** — `scripts/fab-vision-cli.py` (installed/discoverable as `fab-vision`) — that:

1. Satisfies the existing `$AUTOSPEC_FAB_VISION_CMD` contract **exactly** (judge pass + `--verify` pass; same argv and JSON shapes the stage already calls and `test_stage_vision.py` already pins), so `stage_vision.py` needs **no changes**.
2. Reads the render contact sheet, resolves the per-view PNGs it references, and asks **Claude vision** (`claude-opus-4-8` via the Anthropic API, or the `claude` CLI when available) to inspect the renders against the STL Modeling Rules.
3. **Degrades gracefully and never hard-fails a fab run**: no API key, offline, missing `anthropic` SDK, no images on disk, or any backend error → it exits in a way that makes the stage record `status: "skip"` (honest deferral, same as today), never a crash and never a fabricated finding.
4. Is genuinely usable: a named CLI an operator can run by hand, point `$AUTOSPEC_FAB_VISION_CMD` at, and that pays off on the next real fab run — not another shim.

This is the **opposite of an honest deferral**: we ship the consumer. The deferral path remains, but only as the degradation behavior when no backend is reachable.

## Approach / Architecture

### One new script, zero stage changes

Add `skills/autospec-fab/scripts/fab-vision-cli.py` (stdlib + optional `anthropic`). The stage (`stage_vision.py`), the engine (`stl-release-gate.py`, `release_gate_stages.py`), and the render→vision handoff are **untouched**. The new CLI is a drop-in `$AUTOSPEC_FAB_VISION_CMD` target.

How the stage drives it today (load-bearing — the CLI must match these byte-for-byte):

- **Judge pass:** stage runs `<cmd> <sheet> [<rules>]`, expects stdout JSON `{"observations": [ {observation, severity, view?, rule?}, ... ]}`, exit 0. Non-zero or unparseable → stage treats it as zero candidates.
- **Verify pass:** stage runs `<cmd> --verify <sheet> [<rules>]` with one candidate observation as JSON on **stdin**, expects stdout JSON `{"confirmed": <bool>}`, exit 0. Missing/malformed/non-zero → stage drops that observation (conservative).
- `<sheet>` is the contact-sheet HTML path; `<rules>` is an optional STL Modeling Rules file (the engine does **not** currently pass `--rules`, so the CLI must work with `<rules>` absent).

Because the stage already wraps the command in `_run_cmd` (catches `OSError`) and treats any non-zero / unparseable result as "nothing to report," the CLI's degradation story is simple: **on any backend unavailability, the CLI prints `{"observations": []}` (judge) / `{"confirmed": false}` (verify) to stdout and exits 0.**

### Contact sheet → images

The contact sheet is **HTML referencing sibling PNGs** (`stage_render.build_contact_sheet`: `<img src="<basename>.png">` in the sheet's own directory), not an inline image. The CLI:

1. Reads the sheet HTML, extracts `<img src="...">` basenames in document order (deterministic — render writes them in `REQUIRED_VIEWS` table order).
2. Resolves each `src` relative to the sheet's directory; keeps the ones that exist on disk.
3. Caps the number of images sent (default 16, env-overridable) to bound tokens/cost; logs (to stderr) when capping.
4. If **no** referenced PNG exists on disk, the CLI has nothing to inspect → emits the empty-judge degradation and exits 0.

Each resolved PNG becomes a base64 `image` content block (`media_type: image/png`); the rules text (when `<rules>` given) plus a fixed instruction prompt become the `text` block.

### Backend resolution (decided default + override)

First usable wins:

1. **`$AUTOSPEC_FAB_VISION_BACKEND`** — explicit override: `api` | `claude-cli` | `none`.
2. **Anthropic API** (`api`) — if `import anthropic` succeeds **and** a key resolves (`ANTHROPIC_API_KEY` or SDK default). Model `claude-opus-4-8` (vision-capable; **read the `claude-api` skill before editing the call**), base64 PNG image blocks + text prompt, structured-output JSON. The **default real consumer**.
3. **`claude` CLI** (`claude-cli`) — if `claude` is on PATH and supports non-interactive image input (verify support; see Open decisions).
4. **none** — nothing usable → honest skip.

The judge prompt asks Claude to return observations only for things it can actually see, each tagged `severity: info|warn` and an optional `view`/`rule`, and to **return an empty list when nothing is notable** (anti-fabrication). The verify prompt re-shows the renders + the single candidate and asks `{"confirmed": true|false}`.

### The vision-cli contract (authoritative)

**Judge pass:** `fab-vision-cli.py <sheet> [<rules>]` → stdout `{"observations": [{"observation": str, "severity": "info"|"warn", "view"?: str, "rule"?: str}, ...]}`, exit 0. Missing/unreadable sheet → `{"observations": []}`, exit 0.

**Verify pass:** `fab-vision-cli.py --verify <sheet> [<rules>]` with one observation JSON on stdin → stdout `{"confirmed": true|false}`, exit 0. Unreadable stdin / backend error → `{"confirmed": false}`, exit 0.

**Exit codes:** `0` always for reachable/unreachable-backend outcomes (verdict lives in JSON); `2` usage error only (bad flags / no positional `<sheet>`).

**Environment**

| Var | Meaning | Default |
| --- | --- | --- |
| `AUTOSPEC_FAB_VISION_CMD` | (consumed by the **stage**) path/name of this CLI | unset → stage skips |
| `AUTOSPEC_FAB_VISION_BACKEND` | force `api` \| `claude-cli` \| `none` | auto-detect |
| `ANTHROPIC_API_KEY` | API backend credential | unset → API unavailable |
| `AUTOSPEC_FAB_VISION_MODEL` | override model id | `claude-opus-4-8` |
| `AUTOSPEC_FAB_VISION_MAX_IMAGES` | cap images sent | `16` |

### Graceful degradation (the non-negotiable rule)

Every failure mode resolves to **honest skip/empty, exit 0, no crash, no fabricated finding**: CMD unset → stage `skip`; CLI present but no key/SDK/`BACKEND=none` → empty judge → stage `pass` with empty findings; offline/API error/timeout → caught, empty, exit 0; sheet present but zero PNGs → empty; malformed backend JSON → salvage-or-empty; a confirmed warn → `vision_findings` populated, `status` advisory `warn` (never `fail`).

We do **not** add a backend-unavailable→skip channel in this iteration (would need a stage change). Empty-`pass` is an honest "vision ran, nothing actionable / nothing to inspect" — flagged in Open decisions.

## Testing (real-services rule)

Mirror `test_stage_vision.py`: **PATH-stub the vision backend, never mock the stage**. New `tests/test_fab_vision_cli.py` (unittest) + `tests/test_fab_vision_cli.bats`:

1. **CLI-unit (backend=none):** judge `{"observations": []}` exit 0; verify `{"confirmed": false}` exit 0.
2. **Contact-sheet parsing:** generate the fixture via the real `stage_render.build_contact_sheet` (self-consistent-fixture guard — pin against the producer, don't hand-write the HTML the resolver also parses); resolver finds N images in table order; absent PNGs → zero.
3. **Backend stub (integration):** an `AUTOSPEC_FAB_VISION_STUB` seam returns canned judge/verify; assert images/rules pass through and the contract JSON is emitted unchanged. No real Anthropic call in CI.
4. **End-to-end through the unmodified stage (bats):** install the CLI as `fab-vision` in `$TMP/bin`, set CMD + stub backend, run `stage_vision.py`, assert `vision_findings` carries the confirmed observation, `status` `warn`/`pass` (never `fail`). (bats 3.2: write candidate JSON to a real temp file, never `[ -f <(...) ]`.)
5. **Degradation E2E:** `BACKEND=none` → empty findings, `pass`, exit 0; CLI removed from PATH → stage `skip`.
6. **Never-fail invariant:** alarming confirmed warn → `status != "fail"` end-to-end.

Real Anthropic API exercise is **manual/opt-in only** (README), never in unit/bats — keeps validate.sh deterministic/offline.

## File pointers

- Contract being satisfied: `skills/autospec-fab/scripts/stage_vision.py` (`resolve_vision_cmd`, `judge_contact_sheet`, `_confirm_observation`) — **do not modify**.
- Input producer: `skills/autospec-fab/scripts/stage_render.py` (`build_contact_sheet`), `scripts/render_views.py` (`REQUIRED_VIEWS` order) — generate fixtures from `build_contact_sheet`.
- Handoff: `scripts/stl-release-gate.py` (`_vision_in_override`, `run_gate`), `scripts/release_gate_stages.py` (`contact_sheet_for`).
- Test pattern: `tests/test_stage_vision.py` (`$TMP/bin` shim + env-driven fake backend).
- Backend call reference: the `claude-api` skill (base64 image content blocks, `messages.create`, `claude-opus-4-8`) — read before writing the API call.
- README env docs to extend: `skills/autospec-fab/README.md`.

## Decomposition hint (small auto-implement children, sized for a 32B LLM)

1. **`fab-vision-cli` skeleton + contract + degradation (backend=none) + unit tests.** argparse (judge/`--verify`), contact-sheet→PNG resolver (against real `build_contact_sheet`), `none`/unavailable degradation (empty judge/`confirmed:false`, exit 0), `tests/test_fab_vision_cli.py` parsing + degradation. No real backend yet. (Trio note: `scripts/`-only Python + tests — NOT a SKILL.md trio edit — no derive-trio/goldens regen; confirm during implementation.)
2. **Real Claude-vision backend + stub seam + integration test.** `api` backend (anthropic SDK, `claude-opus-4-8`, base64 images, JSON verdict) behind `AUTOSPEC_FAB_VISION_BACKEND`, plus `AUTOSPEC_FAB_VISION_STUB`. Integration + `tests/test_fab_vision_cli.bats` end-to-end through the unmodified `stage_vision.py`. No real API call in CI.
3. **README env-var docs + concrete example + manual smoke note.** Document the 4 env vars; runnable example; graceful-skip behavior; opt-in real-API smoke. Docs-only.

> Children 1+2 must keep validate.sh green on a clean checkout — materialize PNG/sheet fixtures at runtime (call `build_contact_sheet`) or `git add -f` + assert presence (gitignored-fixture guard).

## Open decisions (need operator)

1. **Default backend = Anthropic API (`claude-opus-4-8`) vs `claude` CLI.** Spec defaults to **API** (portable, deterministic auth). Prefer the `claude` CLI as primary where installed? Needs confirmation the installed `claude` supports non-interactive single-shot **image** input; if not, `claude-cli` backend is dropped this iteration and API-only ships.
2. **Honest-skip signal when backend unavailable: empty-`pass` (chosen) vs explicit `deferred` advisory.** Empty-`pass` requires zero stage changes; a distinguishing `deferred` marker needs a small `stage_vision.py` change (out of scope).
3. **Model + cost ceiling.** `claude-opus-4-8`, ~16 images, two passes → non-trivial per-model token cost. Acceptable, or cap to a cheaper vision model / fewer images / verify-only-warns? Overridable via env.
4. **STL Modeling Rules source.** Engine doesn't thread `--rules` to vision today. Wire the rules file into `extra_args_for("vision", ...)` as a fast-follow? Recommended, not blocking.
