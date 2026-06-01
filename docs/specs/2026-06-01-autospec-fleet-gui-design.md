# autospec-fleet GUI design

> Operator-facing GUI for `/autospec-fleet` that lets the user pick repos to
> monitor and edit fleet configuration from a one-page local web app. Repos
> are sourced from `gh repo list` and shown sorted by most recent commit
> (`pushedAt` desc).

## Goal

Add a new `/autospec-fleet gui` subcommand that launches a one-page browser
GUI on `127.0.0.1` with a random port + random URL token, lets the operator
toggle which repos are in the fleet and edit the top-level
`autospec-fleet.yml` keys, and atomically persists their selections back to
`autospec-fleet.yml` in the current workspace.

## Why

Today `/autospec-fleet init <repo-url>...` requires the operator to know
exact repo URLs and type them on the command line. Operators with dozens of
accessible repos can't easily decide which to enroll without paging through
`gh repo list` output. The fleet config (`autospec-fleet.yml`) is also
hand-edited YAML; small typos break `fleet-config-lint.sh`. A simple GUI
sourced from `gh repo list` and a guided form for the top-level config keys
eliminates both of these friction points and makes the most-recently-active
repos surface first.

## Team personality

**Implementation team: Frontend / product engineering.**

- Frontend developer — single-file vanilla HTML/JS app, no framework.
- UX designer — two-column layout, scannable repo rows, sensible defaults.
- Accessibility reviewer — keyboard nav, ARIA labels, semantic HTML.
- API / backend developer — Python `http.server` + bash launcher, JSON
  endpoints with stable shapes.
- QA engineer — round-trip config tests, sort-order tests, fail-mode tests.

Why this team: the user-visible artifact is a GUI. The primary risks are
clarity (operator picks the wrong repos), keyboard accessibility, and
preserving non-managed YAML keys on save.

## Review counter-team

**Review team: Accessibility / API contract / QA.**

- Accessibility reviewer — keyboard-only flow works; focus order is sane;
  search box has an `aria-label`; checkboxes are reachable by Tab.
- API contract reviewer — JSON shapes for `/api/repos` and `/api/config` are
  stable, documented inline in the spec, and round-trippable.
- QA reviewer — exercises empty-repo-list, no-`gh`, gh-not-authenticated,
  missing-config, atomic-write-under-contention paths.

Counter-team mandate: challenge the assumption that "just sort by pushedAt
desc and show checkboxes" is enough. Specifically, probe what happens when
the operator has 200+ repos, when two GUIs run concurrently, and when the
config file already contains keys the GUI doesn't know about.

## Scope

- New subcommand `/autospec-fleet gui` in the lockstep trio.
- New `scripts/fleet-gui.sh` backend launcher (Python stdlib http.server).
- New `gui/index.html` single-file frontend (vanilla JS, no framework).
- New `tests/fleet/test_fleet_gui.bats` covering backend endpoints.
- `scripts/validate.sh` learns a `check_fleet_gui_subcommand_lockstep()`.

## Out of scope

- Editing per-repo `profile` overrides (v2; for now all repos take the
  fleet's `default_profile`).
- Editing the node-local capacity YAML (`~/.autospec/fleet/node-local.yml`)
  — separate file with different access lifecycle.
- A status dashboard during fleet runs — file follow-up issue.
- Authentication beyond the random URL token + 127.0.0.1 bind.

## Architecture

```
operator types `/autospec-fleet gui`
        │
        ▼
skill adapter (SKILL.md / codex/prompt.md / opencode/agent.md)
        │
        ▼
scripts/fleet-gui.sh
   │  picks random port (49152-65535) and random URL token (16 hex)
   │  starts python3 -m http.server (or python stdlib BaseHTTPRequestHandler)
   │  binds 127.0.0.1
   │  opens default browser at http://127.0.0.1:<port>/?t=<token>
   │  on /api/config POST or 15min idle: clean shutdown
   ▼
gui/index.html (served as /)
   ├─ GET /api/repos  → cached `gh repo list ... --limit 200`, sorted desc
   ├─ GET /api/config → reads $CWD/autospec-fleet.yml (or default skeleton)
   └─ POST /api/config → atomic write under flock
```

### Backend (`scripts/fleet-gui.sh`)

Endpoints, all JSON, all require `X-Autospec-Token: <token>` header (token
also accepted in `?t=` for the initial page load).

**`GET /api/repos`**

Calls:
```bash
gh repo list --json nameWithOwner,pushedAt,visibility,description,url \
  --limit 200
```

Returns the array sorted by `pushedAt` descending. Shape (per row):

```json
{
  "nameWithOwner": "berlinguyinca/autospec",
  "pushedAt": "2026-06-01T11:55:00Z",
  "visibility": "PUBLIC",
  "description": "...",
  "url": "https://github.com/berlinguyinca/autospec"
}
```

**`GET /api/config`**

Reads `$CWD/autospec-fleet.yml`. If missing, returns the default skeleton:

```yaml
version: 1
workspace: .autospec-fleet/repos
default_profile: qwen3-32b-laptop
parallel_repos: 2
repos: []
```

Response wraps the YAML in `{ "config": <parsed>, "exists": <bool> }`.

**`POST /api/config`**

Body: full config JSON. Backend:

1. Acquires `flock` on a sentinel file under `$CWD/.autospec-fleet/.gui-lock`.
2. Reads the on-disk YAML if present, preserving any keys not in the GUI's
   managed set (`version`, `workspace`, `default_profile`, `parallel_repos`,
   `repos`).
3. Merges GUI updates over preserved keys.
4. Writes to a temp file in the same directory, then `mv -f` over the
   target (atomic on the same filesystem).
5. Releases flock.
6. Returns `{ "saved": true, "repos_count": N }` and arms a 1-second shutdown.

### Frontend (`skills/autospec-fleet/gui/index.html`)

Single file. No framework. Layout:

- Header: title "autospec-fleet" + workspace path + Save button.
- Left column (35%): config form.
  - workspace (text)
  - default_profile (text — could be enumerated in v2)
  - parallel_repos (number)
- Right column (65%): repo picker.
  - Search box (filters by `nameWithOwner` substring, client-side).
  - Repo list — each row is a `<label>` with a checkbox + name + relative
    pushed-at + visibility badge + description (truncated).
  - "Select all visible" / "Clear all" links above the list.

State management: vanilla `fetch` + `localStorage` for last-used token.
Save: POST `/api/config`, then `window.close()` (best-effort), then show
"Saved — server stopped" if the close fails.

Accessibility:

- Every interactive element has a visible focus ring.
- Search box: `aria-label="Filter repos by name"`.
- Checkbox rows: `<label for="repo-<i>">` wrapping the input.
- Save button: keyboard shortcut Cmd/Ctrl+S.

### Fail modes

| Condition | Behavior |
|---|---|
| `gh` not on PATH | `fleet-gui.sh` prints clear error to stdout and exits 1 with `code_health:fleet_gui_missing_gh` |
| `gh auth status` fails | `/api/repos` returns `{"error":"gh_not_authenticated","hint":"run: gh auth login"}` with HTTP 503; GUI shows the hint |
| `autospec-fleet.yml` malformed | `/api/config` returns the parsed valid prefix + `{"warning":"yaml_partial"}` |
| Two GUIs run concurrently | flock serializes saves; the later POST wins cleanly |
| Browser does not auto-open | `fleet-gui.sh` prints the URL to stdout so the user can paste it |
| 15-min idle (no requests) | `fleet-gui.sh` self-exits with `idle_timeout` and a final log line |

## Implementation outline

Three files do the heavy lifting; the other three are lockstep adapter edits
and tests.

```
skills/autospec-fleet/scripts/fleet-gui.sh    # ~120 LOC bash + embedded python
skills/autospec-fleet/gui/index.html          # ~250 LOC HTML + inline JS + CSS
skills/autospec-fleet/SKILL.md                # +20 lines: gui subcommand
skills/autospec-fleet/codex/prompt.md         # lockstep mirror
skills/autospec-fleet/opencode/agent.md       # lockstep mirror
scripts/validate.sh                            # +check_fleet_gui_subcommand_lockstep
tests/fleet/test_fleet_gui.bats               # 5 tests
```

Each file changes ≤30 lines outside its own creation; new files are
self-contained per the small-LLM sizing rule.

## Tests required

`tests/fleet/test_fleet_gui.bats`:

1. **backend sort order** — stub `gh repo list` returns three repos with
   pushedAt 2026-06-01, 2026-05-28, 2026-05-30 → `/api/repos` returns them
   in order 06-01, 05-30, 05-28.
2. **config round-trip** — POST a config with managed keys + one unmanaged
   key `experimental_thing: 42`; subsequent GET returns both; on-disk YAML
   preserves the unmanaged key verbatim.
3. **missing gh** — `PATH=/usr/bin:/bin` (no `gh`); script exits 1 with
   `code_health:fleet_gui_missing_gh`.
4. **default skeleton** — in an empty directory, GET `/api/config` returns
   the default skeleton with `exists: false`.
5. **flock serialization** — fire 2 concurrent POSTs with different repo
   lists via curl; assert the on-disk YAML is one of the two valid full
   shapes (no half-written file, no JSON-corrupted YAML).

`scripts/validate.sh`: `check_fleet_gui_subcommand_lockstep()` asserts the
literal line `/autospec-fleet gui` appears in all three adapter files
(SKILL.md, codex/prompt.md, opencode/agent.md).

## Acceptance criteria

- [ ] `/autospec-fleet gui` is documented in all three lockstep adapter files.
- [ ] `bash scripts/validate.sh` passes (including new lockstep check).
- [ ] `bats tests/fleet/test_fleet_gui.bats` passes (all 5 tests).
- [ ] `gh repo list` repos appear in `/api/repos` sorted by `pushedAt` desc.
- [ ] Saving the GUI writes back to `autospec-fleet.yml`, preserves
      unmanaged keys, and shuts the server down within 2 seconds.
- [ ] `code_health:fleet_gui_missing_gh` is emitted when `gh` is absent.
- [ ] The HTTP server binds only to `127.0.0.1` and requires the URL token.
- [ ] Idle timeout (15 min, env-tunable via `AUTOSPEC_GUI_IDLE_SECS`) is honored.

## Primary smoke test (inner loop)

```bash
bash skills/autospec-fleet/scripts/fleet-gui.sh --no-browser --print-url --once
```

(In tests: launches the server, prints the URL, hits `/api/repos` and
`/api/config` once, exits 0. The `--once` flag bypasses the 15-min loop.)

### fleet-gui.sh CLI flags

| Flag | Description |
| --- | --- |
| `--no-browser` | Start the server without opening the system browser. |
| `--print-url` | Print the full URL (with token) to stdout before serving. |
| `--once` | Smoke-test mode: start the server, self-issue one GET to `/api/repos` and `/api/config`, assert both return 200, then exit 0. Bypasses the 15-min loop. |

Environment variable: `AUTOSPEC_GUI_IDLE_SECS` — idle timeout in seconds (default 900 = 15 min).

## Operator / full verification

```bash
bash scripts/validate.sh
bats tests/fleet/test_fleet_gui.bats
# Manual smoke:
cd /tmp/empty-fleet-dir && bash /path/to/skills/autospec-fleet/scripts/fleet-gui.sh
# → browser opens → repos sorted most-recent first → tick 3 → Save
# → autospec-fleet.yml has those 3 repos enabled: true
```

## Issue decomposition outline

Four child issues, all `auto-implement`, with one umbrella:

1. **Backend launcher** (`scripts/fleet-gui.sh`) — `ctx:64k`, `reasoning:medium`.
2. **Frontend page** (`gui/index.html`) — `ctx:32k`, `reasoning:medium`.
3. **Skill adapter trio + validate.sh lockstep check** —
   `ctx:64k`, `reasoning:shallow`.
4. **Tests** (`tests/fleet/test_fleet_gui.bats`) — `ctx:32k`, `reasoning:medium`.
   Depends on #1.

## Autonomous assumptions

> AUTONOMOUS ASSUMPTION: The operator wants the GUI to live inside the
> existing `/autospec-fleet` skill rather than a new `/autospec-fleet-gui`
> top-level skill. Existing skill-per-capability rule
> ([[feedback_autospec_skill_per_capability]]) is satisfied because this is
> a sub-mode of fleet configuration, not a new capability surface.

> AUTONOMOUS ASSUMPTION: Python 3 stdlib is acceptable as the only runtime
> dependency (already a transitive autospec dep via mempalace). No pip
> packages introduced.

> AUTONOMOUS ASSUMPTION: Vanilla HTML/JS with no framework is the right
> choice given autospec's minimal-deps ethos.
