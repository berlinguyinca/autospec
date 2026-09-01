# Feedback: Autonomous error recovery — never stall on recoverable tool errors

## Context

During an autospec-run implementer session, two recoverable errors caused
unnecessary stalls that surfaced to the operator:

1. **Duplicate edit pattern**: An exact-text replacement matched 2 locations in
   the file. The correct response is to fix ALL occurrences in one pass (e.g.
   `sed -i` with global flag, or provide more surrounding context to make each
   unique). Never ask "which one did you mean?"

2. **GraphQL rate limit**: `gh pr create` / `gh pr merge` use GraphQL which has
   a separate rate limit from REST. When GraphQL returns "API rate limit already
   exceeded", immediately fall back to the REST API:
   - Create PR: `gh api repos/{owner}/{repo}/pulls --method POST -f title=... -f head=... -f base=...`
   - Merge PR: `gh api repos/{owner}/{repo}/pulls/{n}/merge -X PUT -f merge_method=squash`
   - Check state: `gh api repos/{owner}/{repo}/pulls/{n} --jq '.state'`

## Rule

**Never surface a recoverable error to the operator.** The implementer's
contract is: detect the error class, apply the known recovery, and continue.
Only stop for errors that require a human decision (auth failure, permission
denied on a resource you don't own, destructive action without confirmation).

## Recovery table (implementer must know)

| Error signature | Recovery |
|---|---|
| "Found N occurrences" (edit tool) | Use `sed -i` or fix all occurrences in one pass |
| "GraphQL: API rate limit already exceeded" | Fall back to `gh api` (REST) for the same operation |
| "claim_lost" (exit 2) | Refresh queue, try next candidate |
| "branch already exists" | Adopt the existing branch (worktree `--adopt`) |
| "PR already exists" | Skip implementation, verify + review + merge existing PR |
| "worktree already exists" | `git worktree remove --force` then recreate, or adopt |
| "cargo build timeout" | Retry once with longer timeout; if still failing, check for deadlocks |

## Anti-pattern

Stopping to show the operator the error and asking "what should I do?" is a
contract violation. The operator invoked `/autospec-run` expecting autonomous
operation. The only valid stop condition is a hard blocker that requires a
human decision (e.g. "should I delete this branch?" or "the issue is ambiguous
and could be interpreted two ways").
