# Reviewer contract (autospec Phase 4 fused guardian+LGTM)

This is the **reviewer-relevant extract** of the autospec project rules. It is
the curated subset a reviewer ACTS on — the guardian rubric, the RULE_ID table
the fused reviewer applies, the corrective-directive map, the per-issue opt-out
grammar, and the verdict format. The full ~14KB `autospec-run/SKILL.md`
monitor-loop machinery is intentionally NOT here: as a single-pass reviewer you
audit one PR diff and you do not run the orchestrator's monitor phases, profile
machinery, or invocation flags.

Treat this contract as authoritative for HOW to review one PR. It is the acting
digest for the guardian + LGTM verdict, not a replacement for the per-step
review prompt assembled by `gen-reviewer-prompt.sh`.

## Reviewer standards

- **No verdict without the diff.** Apply every check against the actual PR diff
  and the issue body's declared scope — never approve on prose alone.
- **Honor declared opt-outs.** A valid `Guardian: skip-RULE_ID # <reason>` line
  in the issue body downgrades that RULE_ID to an `INFO:` line; it does NOT block.
- **Enforce TDD, no DB mocks, conventional commits, no unsafe git ops** as
  AGENTS.md `## Engineering standards` requires.
- **Bounded effort.** Max 25 tool calls total across both review parts. If the
  budget is exhausted, append `RULE_ID:OUT_OF_SCOPE: reviewer budget exhausted;
  PR needs human review` and proceed to the verdict.

## Guardian rubric (Part 1 — contract compliance)

Skip Part 1 entirely if `AUTOSPEC_NO_GUARDIAN=1` (log
`WARN: guardian disabled by AUTOSPEC_NO_GUARDIAN`).

1. Read AGENTS.md `## Implementation-quality contract` for the RULE_ID table and
   directive map (the canonical copy; the table below mirrors it).
2. Read the issue body — note `## Implementation scope`, `## Implementation
   outline`, `## Tests required`, and any `Guardian: skip-*` lines.
3. Read deterministic findings from `lint-implementation.sh` output (included in
   the dynamic suffix if present).
4. Apply the **LLM-tier** RULE_IDs against the diff: `HALLUCINATED_API`,
   `DUPLICATE_CODE`, `STRING_MATCH_DOMAIN_LOGIC`, `REPEATED_STRUCTURE_AS_CODE`,
   `DOC_OUT_OF_SYNC` (semantic pass), `INVENTED_CONFIG`. Collect findings as
   `RULE_ID:<path>:<line>: <desc>`. Honor `Guardian: skip-*` with `INFO:` lines.

### RULE_ID table

| RULE_ID | Detector | Tier | Threshold / regex |
|---|---|---|---|
| `OUT_OF_SCOPE` | det | exact/prefix path compare | files touched ∉ exact files or trailing-slash directories declared in `## Implementation outline` ∪ `## Files touched` |
| `MISSING_TEST` | det | path-prefix scan | required test type from issue body `## Tests required` not present in diff under `tests/{unit,integration,smoke,e2e}/` |
| `COMPLEXITY` | det | line/regex scan | function >50 LOC, file >500 LOC, nesting >4 |
| `SECURITY` | det | regex match | `eval\(`, `exec\(`, `--no-verify`, `git reset --hard`, `rm -rf /`, AWS-key shape `AKIA[0-9A-Z]{16}`, GitHub-token shape `gh[pousr]_[A-Za-z0-9]{36,}`, private-key markers `-----BEGIN [A-Z ]*PRIVATE KEY-----`, or `localStorage` / `sessionStorage` / `document.cookie` use involving token, API key, credential, auth, authorization, bearer, or group state |
| `TODO_LEFT` | det | regex on non-test diff | `\b(TODO\|XXX\|FIXME)\b` |
| `MOCK_DB` | det | regex on test diff | `\b(mock\|stub)\b` near DB-symbol heuristics (`db\.`, `database`, `DataSource`, `pg`, `mysql`, `sqlite`) |
| `HALLUCINATED_API` | LLM | semantic | symbol referenced in diff not defined in diff, not in pre-PR repo (verifiable via repo search), not in dependency manifests |
| `DUPLICATE_CODE` | LLM | semantic | new code mirrors an existing helper (must cite `<path>:<line>`) |
| `STRING_MATCH_DOMAIN_LOGIC` | LLM | semantic | code uses substring checks against free-form text to encode domain meaning, AND a proper-representation library is imported in the file |
| `REPEATED_STRUCTURE_AS_CODE` | LLM | semantic | ≥5 branches in the same function/method sharing identical structural shape (same return shape, predicate signature, side-effect line) |
| `DOC_OUT_OF_SYNC` | hybrid | det+LLM | det: any change to public surface (CLI flag, env var, exported function, config key) WITHOUT a touched doc file (`README*`, `AGENTS.md`, `docs/**`, `SKILL.md`); LLM: judges semantic accuracy when a doc IS touched |
| `INVENTED_CONFIG` | LLM | semantic | flag/env-var/config-key introduced in diff not present in issue body or referenced spec |
| `PR_SIZE` | det | git diff/numstat | hard above 400 additions+deletions, 8 raw files, or 3 normalized logical units; binary rows are always hard |

### Corrective directive map

When a RULE_ID fires, the finding carries this single-line corrective directive
so the implementer knows the fix on retry:

| RULE_ID | Directive |
|---|---|
| `OUT_OF_SCOPE` | "Restrict the diff to exact files or descendants of trailing-slash directories declared in `## Implementation outline` or `## Files touched`. Revert undeclared files; incomplete scope must be corrected by the issue author." |
| `MISSING_TEST` | "Add a test under tests/<TIER>/ for the listed required test type before re-pushing." |
| `COMPLEXITY` | "Split functions >50 LOC, files >500 LOC, nesting >4. No copy-paste branches." |
| `SECURITY` | "Remove the flagged pattern. NEVER hardcode secrets, NEVER use --no-verify or git reset --hard, validate input at boundaries, and do not persist token/API-key/auth/group authorization state in browser storage without an explicit scoped security decision." |
| `TODO_LEFT` | "Remove TODO/XXX/FIXME from non-test code. File a follow-up issue if the work is genuinely deferred." |
| `MOCK_DB` | "Remove DB mock/stub. Use the real DB per AGENTS.md ## Engineering standards." |
| `HALLUCINATED_API` | "The flagged symbol does not exist. Verify identifier names against the pre-PR repo and dependency manifests." |
| `DUPLICATE_CODE` | "Reuse the existing helper at <path>:<line> instead of re-implementing." |
| `STRING_MATCH_DOMAIN_LOGIC` | "Replace substring checks with the proper domain primitive (AST/parsed URL/IP/date/schema)." |
| `REPEATED_STRUCTURE_AS_CODE` | "Extract the N branches into a table + single dispatcher loop." |
| `DOC_OUT_OF_SYNC` | "Update the doc file(s) covering the changed public surface in this same PR." |
| `INVENTED_CONFIG` | "Remove the invented flag/env/key, or amend the issue body to introduce it as scope." |
| `PR_SIZE` | "Freeze the completed capped slice and move unmet acceptance criteria to ordered continuation issues; never push or merge this oversized diff." |

## Per-issue opt-out grammar

The issue body MAY declare per-RULE_ID opt-outs with mandatory justification:

```
^Guardian:\s+(skip-[A-Z_]+(,\s*skip-[A-Z_]+)*)\s+#\s+\S.+$
```

- Justification (text after `#`) is **mandatory**. Bare `Guardian: skip-X` is
  rejected (malformed; RULE stays active).
- Skipped RULE_IDs are emitted as `INFO:RULE_ID...` (audit trail) but do NOT
  block the merge.
- Skips apply only to the PR derived from this issue; they do NOT cascade.

## LGTM rubric (Part 2 — correctness review)

5. Check correctness, edge cases, missing tests, and AGENTS.md compliance (TDD,
   no mocks, conventional commits).
6. Collect findings as a numbered list.

## Verdict

If Part 1 has ZERO blocking findings (`INFO:` lines are OK) AND Part 2 has no
findings: return ONLY the token `LGTM`. Otherwise return a numbered findings
list — RULE_ID findings first, then LGTM findings.
