# CSV schema and lifecycle

Two files, identical schema:

- **Per-run snapshot:** `reports/autospec-review/<YYYY-MM-DD>-<run_id>.csv`
- **Append-only ledger:** `reports/autospec-review/gaps.csv`

`run_id` = `<UTC ISO compact>-<short_git_sha>` (e.g. `20260506T1430Z-6c2e3a4`).

## Columns (frozen for v1)

| # | Name | Type | Description |
|---|------|------|-------------|
| 1 | `gap_id` | str | `sha1(spec_path + spec_anchor + gap_type)[:10]` — primary key |
| 2 | `run_id` | str | run when row was written |
| 3 | `audit_date` | ISO date | yyyy-mm-dd |
| 4 | `repo` | str | `owner/name` |
| 5 | `spec_path` | str | repo-relative |
| 6 | `spec_topic` | str | filename slug (date-prefix stripped) |
| 7 | `gap_type` | enum | `ac_no_issue` \| `closed_missing_code` \| `closed_unchecked_ac` \| `section_no_coverage` |
| 8 | `severity` | enum | `blocker` \| `major` \| `minor` |
| 9 | `title` | str | short imperative |
| 10 | `spec_anchor` | str | section heading (verbatim) |
| 11 | `evidence` | str | ≤ 500 chars, newlines escaped to `\n` |
| 12 | `suspected_issues` | str | space-separated `#nnn` list |
| 13 | `remediation_issue` | str | `#nnn` once filed; empty initially |
| 14 | `remediation_pr` | str | `#nnn` once /autospec-run merges |
| 15 | `status` | enum | `open` \| `filed` \| `fixed` \| `wontfix` \| `false_positive` |
| 16 | `notes` | str | free text; user-editable; preserved across runs |

## Lifecycle

1. New gap → `status=open`.
2. After `/autospec-split` returns issue numbers → write `remediation_issue`,
   flip to `status=filed`.
3. On future runs, key by `gap_id`. If existing `status=filed` row's evidence
   no longer reproduces (artifact present, AC checked, etc.), flip to
   `status=fixed` BEFORE generating regression spec — so we don't refile.
4. Manual `wontfix` / `false_positive` are preserved across runs.
   Excluded from regression-spec generation.
5. A regression issue closed-without-merge on a later audit → flip back
   to `status=open` with note `Re-opened on <run_id>: closed issue
   #<nnn> was not merged`.

## CSV escaping rules

- `evidence` column: replace literal `\n` with `\\n` AND `\r` with `\\r`
  AND `"` with `""` (CSV double-quote convention). Truncate at 500 chars
  AFTER escaping; append `…` if truncated.
- All other str columns: standard CSV double-quote convention.
- Always emit headers row.
- File is UTF-8, LF line endings.

## Append vs replace logic

- Per-run snapshot: replace if same `run_id` already exists; otherwise
  fresh write.
- Ledger: read existing, merge by `gap_id`, write atomically (write to
  `.tmp` then `os.rename`). Existing manual annotations
  (`status` ∈ {wontfix, false_positive} OR `notes` ≠ "") are preserved
  — script must not overwrite these columns.
