# autospec-continue — bridge conversational recommendations into autospec execution

## Summary

A new top-level skill `/autospec-continue` (alias `/continue`) that extracts
the actionable recommendation from the most recent assistant message in the
current conversation, pipes it through `/autospec-refine` (the 4-lens
refinement pipeline shipped via PR #678), and hands off to `/autospec
--autonomous` for end-to-end implementation.

The point: any time the model just said "here's what to do next", the
operator types `/continue` and the model's own suggestion becomes an
autospec loop, with no copy-paste and no manual prompt rewriting.

## Team personality

- **Selected team:** Core product engineering — product manager, architect,
  backend developer, technical writer.
- **Why this team fits:** UX-facing skill that bridges conversational input
  (untrusted prose) into an autonomous code-shipping pipeline. Needs balanced
  product judgment (what counts as actionable), engineering rigor (extraction
  + handoff plumbing), and writing craft (the failure messages ARE the UX).
- **Risks this team will notice:** false extractions (chitchat treated as a
  prompt), prompt injection via the prior assistant message, runaway
  invocations (/continue → /autospec → next message has another action item →
  /continue again).
- **Carry into child issues:** extraction is loud-on-empty; injection-shaped
  content in the extracted block is sanitized before refine handoff;
  rate-limit chained /continue invocations.

## Review counter-team

- **Selected counter-team:** Security + maintainability — security reviewer,
  maintainer, regression-test engineer.
- **Why this counter-team:** the skill reads the prior assistant message
  (which an attacker could influence via prompt injection in upstream data),
  passes it through refine, and triggers autonomous code execution. Trust
  boundary is "anything an assistant said". Maintainability angle catches
  knobs creep (multiple extraction modes, multiple lens-mode passthroughs).
- **What this team should challenge:** can an attacker craft a prior message
  that makes /continue extract a malicious prompt? Does the autonomy gate
  (PR #664) catch destructive intent in the refined output? Are the
  conversation-reading mechanics testable without a live harness?

## Architecture

`autospec-continue` mirrors the existing autospec-refine skill-family shape:

- `skills/autospec-continue/SKILL.md` — Claude Code adapter (authoritative).
- `skills/autospec-continue/codex/prompt.md` — Codex CLI mirror (lockstep).
- `skills/autospec-continue/opencode/agent.md` — OpenCode mirror (lockstep).
- `skills/autospec-continue/install.sh` — installer.
- `skills/autospec-continue/uninstall.sh` — uninstaller.
- `skills/autospec-continue/README.md` — usage doc.
- `scripts/extract-conversational-recommendation.sh` — extraction helper.

## Invocation

```
/autospec-continue [--skip-refine] [--ask-confirm] [--lens-mode deterministic|llm]
                   [--from-message <path>]
```

- `--skip-refine` — bypass `/autospec-refine`; hand the extracted prompt
  directly to `/autospec --autonomous`.
- `--ask-confirm` — after refine but before handoff, surface the refined
  prompt and ask the operator to approve. Defaults to auto-execute.
- `--lens-mode deterministic|llm` — passthrough to `/autospec-refine`'s
  same flag. Default: matches refine's auto-detect.
- `--from-message <path>` — read the source message from a file instead of
  the harness's "last assistant turn". Test/integration path; not the
  primary use case.

## Extraction contract

The skill prose tells the LLM:

1. **Read the most recent assistant message in this conversation.** Each
   harness exposes the message history differently:
   - Claude Code: the orchestrator agent already has the conversation in
     context; the skill simply tells the LLM to look at the prior turn.
   - Codex CLI: same conversation-context access.
   - OpenCode: same.
   - **Test/integration path:** `--from-message <path>` overrides the
     harness mechanism with file content.
2. **Apply the extraction matchers in priority order:**
   - **Section headings (highest priority):** `## Next steps`, `## What to
     do next`, `## Remaining work`, `## Open blockers`, `## Suggestions`,
     `## Proposed follow-ups` (case-insensitive). Extract everything from
     the heading to the next `##` heading or end of message.
   - **Fenced blocks:** ` ```autospec-next ` or ` ```next-prompt ` fenced
     code blocks. Extract the block body verbatim.
   - **Numbered lists with action verbs:** numbered lists where ≥50% of
     items start with imperative verbs (`fix`, `add`, `implement`,
     `update`, `review`, `refactor`, `ship`, `merge`, etc.). Extract the
     whole list.
   - **"You should / I suggest" patterns:** sentences starting with
     `you should`, `I suggest`, `I recommend`, `next step is`, `the next
     thing to do is`. Extract that sentence plus the following paragraph.
3. **Combine matches** (operator confirmed default: pass everything as one
   prompt). Refine's sizing lens splits into multiple issues if needed.
4. **Empty-recommendation case:** if NO matcher hits, exit with
   `code_health:continue_no_recommendation` and the operator-facing
   message: `"No actionable recommendation found in the last assistant
   message. Provide an explicit prompt: /continue \"<your prompt>\"."`

## Prompt-injection guard

The extracted block has been generated by an LLM in this same conversation,
which means an attacker who has influenced earlier turns can plant a
malicious prompt for /continue to extract. Before handing off to refine:

1. Reject extracted blocks containing `ignore previous`, `disregard prior`,
   `you are now`, `system prompt`, or shell metacharacters (`$(`, `` `... ` ``,
   `&&`, `;`, `|`, `>`) at the start of a line. Exit 3 with
   `code_health:continue_injection_detected`.
2. The autonomy gate from PR #664 (`scripts/autospec-autonomy-gate.sh`)
   continues to apply downstream — destructive-action patterns surface for
   confirmation regardless of how the prompt got into the pipeline.
3. The path allowlist from PR #685 continues to apply to any file paths
   the extracted prompt references.

## Rate limiting and runaway protection

Per-invocation, the orchestrator writes `~/.autospec/continue-history.json`
with `{timestamp, source_message_hash, extracted_prompt_hash}`. Before each
invocation:

1. If the same `source_message_hash` was processed in the last 60 seconds
   → exit 3 with `code_health:continue_recent_duplicate` to prevent runaway
   loops (e.g., the operator double-tapped `/continue`).
2. If the same `extracted_prompt_hash` has been processed 3 or more times
   in the last 60 minutes → exit 3 with `code_health:continue_oscillation`
   to prevent feedback loops.

Operator escape: `~/.autospec/continue-history.json` can be deleted to
reset the rate-limit state.

## Error handling

- **Empty case** — exit 3 with `code_health:continue_no_recommendation`.
- **Injection detected** — exit 3 with
  `code_health:continue_injection_detected`, log the offending pattern
  category (not the full extracted block) to stderr.
- **Rate limit** — exit 3 with the appropriate `code_health:continue_*`
  category.
- **Refine fails** — exit propagates refine's exit code; the artifact
  documents the extraction step succeeded but refine did not.
- **Handoff fails** — exit propagates the handoff's exit code; consistent
  with refine's own failure semantics (PR #687).

## Testing

- `tests/continue/test_continue_extract.bats` — fixtures for each matcher:
  section heading, fenced block, numbered list, you-should pattern,
  multi-matcher message, empty message, chitchat-only message.
- `tests/continue/test_continue_injection.bats` — prompt-injection rejection
  for each injection pattern.
- `tests/continue/test_continue_rate_limit.bats` — duplicate within 60s,
  oscillation within 60min, reset after history deletion.
- `tests/continue/test_continue_handoff.bats` — `--skip-refine` routes
  directly to autospec; default route goes through refine; `--ask-confirm`
  gates correctly.

## Acceptance

- New skill family `autospec-continue` ships per the scaffold; passes
  `check_lockstep`.
- `scripts/extract-conversational-recommendation.sh` ships executable.
- `autospec validate` gains `check_autospec_continue_contract()`
  enforcing trio lockstep + helper presence + bats suite.
- All bats fixtures pass.
- End-to-end test: a fixture conversation file whose last "assistant"
  message contains a `## Next steps` section produces a refined-and-
  executed pipeline run.

## Decomposition into child issues

Aiming for 4 children plus an umbrella, sized per the small-LLM rule.

1. **Issue A — skill scaffold**: `skills/autospec-continue/` trio +
   `install.sh` + `uninstall.sh` + `README.md`. Includes the structural
   sections required by the autospec-decomposer contract (Self-update +
   Stop mode + Required capabilities & harness adapter row + Model tier).
   Files: 6.
2. **Issue B — extraction helper**: `scripts/extract-conversational-
   recommendation.sh` implementing the 4 extraction matchers + the
   empty-case failure path + the prompt-injection guard. Depends on A.
   Files: 2 (script + bats).
3. **Issue C — orchestrator + handoff**: extends `scripts/extract-
   conversational-recommendation.sh` (or a sibling orchestrator script —
   implementer's call) to wire extraction → optional refine → handoff to
   `/autospec --autonomous`. Implements `--skip-refine`, `--ask-confirm`,
   `--lens-mode`, `--from-message` flags. Depends on B. Files: 2.
4. **Issue D — rate limit + validate.sh check + e2e**: rate-limit
   bookkeeping (`~/.autospec/continue-history.json`), runaway protection,
   `check_autospec_continue_contract()` in `autospec validate`, and the
   end-to-end bats fixture. Depends on C. Files: 2.

Total: 4 children + 1 umbrella. Each child is well under the autospec
sizing caps.

## Out of scope (defer)

- Cross-conversation `/continue` (reading prior sessions' messages).
- Walking further back than the immediately previous assistant message
  on empty match (the recommended path is fail-loudly).
- Auto-detecting that the operator probably wants `/continue` without an
  explicit invocation.
- Operator-defined extraction patterns (custom matchers).
