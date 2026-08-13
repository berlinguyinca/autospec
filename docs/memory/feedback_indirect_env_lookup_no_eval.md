---
name: feedback_indirect_env_lookup_no_eval
description: "for indirect env-var lookup use bash ${!var:-} not eval; ${!var} IS bash 3.2 safe and eval on config-derived names is command injection"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: d87fae41-0795-45c0-9afe-909bd9bc37fb
---

To read an env var whose NAME is held in another variable (e.g. a config field
`token_env: "GITHUB_TOKEN"`), use bash indirect expansion:

```bash
tok="${!tok_env:-}"
```

NOT `eval`:

```bash
tok="$(eval "printf '%s' \"\${$tok_env:-}\"")"   # WRONG — injection primitive
```

**Why:** the eval form splices the config-controlled name unescaped into a string
that eval re-parses as a shell command. A crafted `token_env` of
`x}"; touch /tmp/pwned #` executes arbitrary commands. Even when the config is
operator-authored (same trust boundary as the scripts), there is zero reason to
keep the risk when a strictly safer equivalent exists.

**How to apply:** `${!name}` indirect expansion is bash 2.0+, so it IS bash 3.2
compatible — I wrongly prescribed the `eval` form in a plan assuming `${!var}`
wasn't 3.2-safe. It is. `${!name:-}` (indirect + default) also works in 3.2.57
(verified). Only `${var,,}` and associative arrays are the real bash-4 hazards;
`${!var}` is fine. Bit the grow-run adapters (Plan 4). When writing/decomposing
any script that reads a var-named-by-a-var, use `${!name:-}`. Related:
[[feedback_jq_test_regex_metachar_injection]] (the same don't-splice-untrusted-
input-into-an-interpreter lesson, jq edition).
