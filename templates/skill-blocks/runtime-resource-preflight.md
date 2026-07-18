> 1a. **Compose normalization and isolated runtime preflight.** Run the Rust
>    normalization check after `worktree-guard.sh assert` and before any dev server,
>    E2E server, database, Docker Compose stack, or browser evidence run. This check is
>    unconditional: do not first test whether a runtime manifest exists.
>    ```bash
>    NORMALIZE_PLAN=$(mktemp "${TMPDIR:-/tmp}/autospec-compose-plan.XXXXXX")
>    NORMALIZE_ERROR="${NORMALIZE_PLAN}.err"
>    if ! autospec runtime env normalize-compose --repo "$PWD" --check >"$NORMALIZE_PLAN" 2>"$NORMALIZE_ERROR"; then
>      if grep -q 'NORMALIZE_COMPOSE_NOT_FOUND' "$NORMALIZE_ERROR"; then
>        : # No Compose prerequisite is present.
>      else
>        sed -n '1,120p' "$NORMALIZE_ERROR" >&2
>        rm -f "$NORMALIZE_PLAN" "$NORMALIZE_ERROR"
>        exit 1
>      fi
>    fi
>    ```
>    Read a successful plan's `edits` and `remaining_diagnostics` fields. Diagnostics
>    fail closed. Non-empty edits require the `autospec-compose-normalize` skill in
>    **Autospec-managed invocation** mode; wait for its fingerprinted prerequisite PR
>    to merge, update this worktree from `origin/main`, and rerun the check. Do not run
>    `env up` until the plan is safe and contains zero edits. Delete the temporary files.
>
>    If `.autospec/runtime.yml` or `.agent-runtime.yml` then exists, provision through
>    the shared broker. Never hand-pick fixed ports. Source the emitted env file so
>    `AUTOSPEC_PUBLIC_URL`, `AGENT_PUBLIC_URL`, `AGENT_FRONTEND_PORT`,
>    `AGENT_BACKEND_PORT`, and `COMPOSE_PROJECT_NAME` flow into all child processes.
>    ```bash
>    rm -f "$NORMALIZE_PLAN" "$NORMALIZE_ERROR"
>    if [ -f ".autospec/runtime.yml" ] || [ -f ".agent-runtime.yml" ]; then
>      autospec runtime env up --repo "$PWD" --mode "${AUTOSPEC_RUNTIME_MODE:-auto}" > .autospec-agent-env.out
>      AGENT_ENV_FILE=$(sed -n 's/^AGENT_ENV_FILE=//p' .autospec-agent-env.out | tail -n 1)
>      if [ -n "$AGENT_ENV_FILE" ] && [ -f "$AGENT_ENV_FILE" ]; then
>        . "$AGENT_ENV_FILE"
>      else
>        echo "autospec runtime env did not emit a usable AGENT_ENV_FILE" >&2
>        exit 1
>      fi
>      echo "[runtime] isolated environment: ${AUTOSPEC_PUBLIC_URL:-$AGENT_PUBLIC_URL}"
>    fi
>    ```
>    Use `AUTOSPEC_PUBLIC_URL` as the canonical browser/QA URL instead of a fixed
>    localhost port.
