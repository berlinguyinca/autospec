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
| `GATE_PROMOTION_UNEVIDENCED` | det | workflow diff scan | a `.github/workflows/*.yml` change promotes a job to a blocking gate (adds it to another job's `needs:` or removes `continue-on-error: true`) without a cited green run (GitHub Actions run/job URL or exit status 0) in the issue or PR body; finding names the file and job |
| `BATS_SUITE_UNREGISTERED` | det | pre-commit path scan | staged `.bats` file added under `tests/unit/` or `tests/lint/` whose quoted path appears in neither `crates/autospec-core/src/validation/catalog.rs` nor `BATS_REGISTRATION_BASELINE` in `crates/autospec-core/src/validation/external/bats_registration_baseline.rs`; suites at `tests/` root are exempt — the authoritative scan is `run_bats_suite_registration` at conversion (#3919) |
| `COMMAND_NOT_REGISTERED` | det | pre-commit staged-diff scan | a new command name introduced to the `COMMANDS` table or the dispatch match in `crates/autospec-cli/src/commands/mod.rs` (new = staged name set minus base name set) whose remaining registration sites — the `COMMANDS` table entry, the dispatch match arm, or the `\`autospec <name> ...\`` row in `docs/cli-reference.md` — are not also staged in the same commit; the finding names every unvisited site with file:line and the value to add (#3964, repro #3793) |
| `CATALOG_ENTRY_INCOMPLETE` | det | pre-commit staged-diff scan | a new catalog check id (new = staged id set minus base id set) that is only half-registered: present in `STANDARD_CHECK_IDS` (`crates/autospec-core/src/validation/catalog/catalog_ids.rs`) without a match arm in `ValidationCheck::catalog_entry` (`crates/autospec-core/src/validation/catalog.rs`, dead code), or present as a match arm without the id (runtime panic, #3964) |
| `UNWIRED_PUB_ITEM` | det | pre-commit staged-diff scan + rg | an added `pub fn`/`pub struct` in a non-test `.rs` diff file with no word-boundary reference outside its own `#[cfg(test)]` module — a well-tested library nothing calls passes every other gate; a reference in another file (call site or `pub use` re-export) counts as wiring, a bare `mod NAME;` declaration does not. The PR says which it is: forgotten wiring (add the caller) or deliberate staging (`Guardian: skip-UNWIRED_PUB_ITEM # <reason>` / `# linter:allow-UNWIRED_PUB_ITEM <reason>`, justification mandatory) (#4346) |

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
| `GATE_PROMOTION_UNEVIDENCED` | "Cite a green run of the promoted job in the issue or PR body (a GitHub Actions run/job URL or a captured exit status 0) before promoting it to a blocking gate, or revert the promotion. If the verification could not be executed, the implementer must record the command and why it could not run in the Closeout report and the PR body."
| `BATS_SUITE_UNREGISTERED` | "Register the new bats suite as a typed ExternalCheck::BatsSuite owner in crates/autospec-core/src/validation/catalog.rs, or add its path to BATS_REGISTRATION_BASELINE in crates/autospec-core/src/validation/external/bats_registration_baseline.rs; suites at tests/ root need no registration." |
| `COMMAND_NOT_REGISTERED` | "Visit every registration site the finding names for the new command: the COMMANDS table entry and the dispatch match arm in crates/autospec-cli/src/commands/mod.rs, plus the \`autospec <name> ...\` row in docs/cli-reference.md — all in this commit." |
| `CATALOG_ENTRY_INCOMPLETE` | "Keep the two catalog sites in lockstep: the id must appear in STANDARD_CHECK_IDS (crates/autospec-core/src/validation/catalog/catalog_ids.rs) and have a match arm in ValidationCheck::catalog_entry (crates/autospec-core/src/validation/catalog.rs) — add the missing one in this commit." |
| `UNWIRED_PUB_ITEM` | "Show the caller that wires the new pub item into a live path, or declare it deliberate staging with `Guardian: skip-UNWIRED_PUB_ITEM # <reason>` (or the inline `# linter:allow-UNWIRED_PUB_ITEM <reason>` hatch) so the diff says which it is. A test-module-only reference is not wiring." |

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
6. For anything touching authentication, signing, encryption, or credential
   handling: ask "Is this crypto/dependency defending against a party who can
   actually reach this data path, or is it re-proving what the transport
   already proved?" If TLS to a named host, a process boundary, or an existing
   gate already establishes the property, flag the re-establishment. The
   reverse also holds: if a token could arrive by another route (forwarded by
   a client, read from a header), it must be verified properly, and that
   boundary must be pinned by a comment at the code.
7. Collect findings as a numbered list.

## Verdict

If Part 1 has ZERO blocking findings (`INFO:` lines are OK) AND Part 2 has no
findings: return ONLY the token `LGTM`. Otherwise return a numbered findings
list — RULE_ID findings first, then LGTM findings.
