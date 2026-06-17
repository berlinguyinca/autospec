---
name: feedback_bash32_process_sub_test_file
description: "macOS bash 3.2 fails `[ -f <(...) ]` process substitution; bats tests that pass a script's output through a `--validate-file`/`[ -f ]` helper must write to a real temp file first"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: ba194365-30f2-4add-918a-ed7263b0d5dd
---

On macOS the system bash is 3.2.57. Process substitution `<(...)` yields a
`/dev/fd/N` path that bash 3.2's `[ -f "$path" ]` test reports as **false**, so
any helper guarded by `[ -f "${2:-}" ]` (e.g. `gap-json-lib.sh --validate-file`)
rejects a process-substitution argument regardless of content. This is a
test/platform incompatibility, not a SUT bug.

**Why:** autospec bats tests run under the host's `/bin/bash` (3.2), and several
scripts validate a file path with `[ -f ]`. A test like
`run bash "$LIB" --validate-file <(printf '%s' "$line")` fails 100% on macOS.

**How to apply:** in bats, write the value to a real temp file and validate that:
`printf '%s' "$line" > "$TMP/x.json"; run bash "$LIB" --validate-file "$TMP/x.json"`.
One-off `<(...)` in interactive verification commands or in LLM-runtime prompt
examples is fine — the rule is only for committed shell that must run under
bash 3.2 (test scripts, the scanner engine). Mind subshell variable loss in
`... | while read` too (use EXIT not RETURN traps). Related: [[feedback_bash_return_trap_leak]], [[feedback_bash_set_e_short_circuit]].
