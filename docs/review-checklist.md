# Review checklist over a set of changed files

A change that is checked only for what it *added* still ships the three defects
that this module exists to catch (issue #231): a config key that does not exist
in the schema, a build step that aborts the build, and a workflow gate that
needs a job it cannot reach. The fix is not a smarter single check — it is a
**checklist**, one row per changed file, the checks that apply to that file's
type, and the comments and config values the change deleted from under it,
handed to the reviewer before they merge.

`autospec_core::review_checklist` is a pure library module. It reads the diff
model and returns a report; it owns no I/O, holds no state, and never fails.
It is not wired to a CLI command.

## Entry points

| Function | Purpose |
|---|---|
| `parse_unified_diff(patch: &str) -> Vec<ChangedFile>` | Turn unified-diff text into the file model, in patch order. |
| `build_review_checklist(&[ChangedFile]) -> ReviewChecklist` | Build the checklist: one entry per file plus all removals. |
| `classify_file_type(path: &str) -> FileType` | Map a path to the file type its checks are chosen from. |
| `checks_for(file_type: FileType) -> &[&str]` | The checks that apply to a file type (empty for prose). |
| `collect_removals(&[ChangedFile]) -> Vec<RemovalItem>` | The removed comments and config values, with adjacency. |
| `is_complete(&ReviewChecklist, &[ChangedFile]) -> bool` | Re-check completeness from the inputs, not the report. |

The report itself is `ReviewChecklist { files, removals, complete }`, where
each `FileCheckEntry` carries `path`, `file_type`, `checks`, and
`no_applicable_check`. Both `to_json()` and `to_text()` are available for
rendering.

## The checks table

Each file type maps to a fixed, ordered set of checks. The mapping is a table —
one row per type — so adding a type or a check is a data change, not a code
change.

| File type | Checks (in review order) |
|---|---|
| TypeScript | every config key exists in the resolved schema; no new object is typed as a catch-all that swallows unknown fields |
| Containerfile | each shell step must not abort the build; every path a step references must exist after that step runs |
| Workflow | every `needs` entry names a job that exists in this workflow; no job needs a job it cannot reach |
| Rust | build is green with no new warnings; new behavior has a test |
| Python | no syntax errors and the linter is clean; new behavior has a test |
| Shell | strict mode is respected and no expansion is left unquoted; every referenced file and variable exists |
| Config | every key exists in the schema the consumer validates against; no removed key is still referenced elsewhere |
| Markdown | *(none — flagged, not dropped)* |
| Other | *(none — flagged, not dropped)* |

A type with nothing mechanical to check (prose, or a format the module does not
model) yields an empty check list. Its entry is still present and carries
`no_applicable_check: true`, so the file is **reported as uncheckable** rather
than silently skipped.

## How a path becomes a file type

Well-known names are decided by basename *before* any extension, because a
`Dockerfile` or a CI file carries no extension to read:

- a path containing `/.github/workflows/` (case-insensitive) → Workflow;
- basename `Dockerfile`, `Containerfile`, or either with a build-variant
  suffix (e.g. `Dockerfile.prod`) → Containerfile;
- basename `docker-compose.yml`/`.yaml` → Config;
- basename `ci.yml`/`ci.yaml` → Workflow;
- basename `Makefile`/`Justfile` → Other.

Otherwise the extension decides: `.ts/.tsx/.mts/.cts` → TypeScript; `.rs` →
Rust; `.py/.pyi` → Python; `.sh/.bash/.zsh/.fish` → Shell; `.md/.markdown` →
Markdown; `.yaml/.yml/.json/.toml/.ini/.conf/.cfg` → Config; anything else →
Other.

## What gets surfaced as a removal

The checklist walks the **removed** lines of each file and keeps the two kinds
a reviewer should see — a comment and a config value — dropping everything
else. Each kept removal records whether it sat **adjacent** to an added line
(within `ADJACENCY_WINDOW = 3` diff-line indices), so a deletion is flagged for
how much the reviewer should lean on it:

- **Comment** — the line starts with the file's language comment marker.
  C-like languages (TypeScript, Rust) use `//` and `/*`; a leading `#` is an
  attribute in Rust and a private field in TypeScript, so it is not a comment
  there. Python, Shell, Markdown, Containerfile, Workflow and Config use `#`.
  The catch-all `Other` recognises all three.
- **Config value** — a `key: value` or `key = value` pair where the key is a
  plain name and the value is non-empty; a trailing `;` on the `=` form marks a
  code statement, not a config line.

Removals are in file order, and within a file in old-line order.

## Two invariants

Every checklist this module produces holds both:

- **It is complete.** `build_review_checklist` emits an entry for *every*
  changed file, in the order given, and sets `complete` from that. Finding a
  defect in one file does not stop the walk, so the checklist never ends at the
  first bad file. `is_complete` re-derives this from the inputs (entry count
  and per-position path equality), so a report built elsewhere can be checked
  the same way.
- **It surfaces what was removed.** Every removed comment and config value is
  carried in `removals`, each flagged for adjacency.

## Using it

```rust
use autospec_core::review_checklist::{build_review_checklist, parse_unified_diff};

let files = parse_unified_diff(patch_text);
let checklist = build_review_checklist(&files);

assert!(checklist.complete);
println!("{}", checklist.to_text());
```

`to_text()` prints one line per file — `path [type]: checks; …` with a
`(no applicable check)` flag where applicable — followed by one line per
removal: `removed <kind> path@<line> adjacent: <content>`. `to_json()` emits
the same report as compact JSON for machine consumers.
