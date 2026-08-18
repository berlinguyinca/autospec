## Startup self-update

The self-update logic lives in `scripts/autospec-startup-self-update.sh`, not in this
block. Shell inlined into a skill body is rendered by the harness before it runs, and a
harness substitutes positional parameters (`$1`) in a rendered body with the caller's
slash-command argument — which previously let a skill argument overwrite an arbitrary
file (issue #3177). This block therefore only resolves and invokes that script.

```bash
SKILL_NAME={{SKILL_NAME}}   # per-skill provenance label; inert, never used as a path
AUTOSPEC_SELF_UPDATE_SCRIPT=""
for _candidate in \
    "${SCRIPT_DIR:-}/autospec-startup-self-update.sh" \
    "${AUTOSPEC_REPO_ROOT:-$PWD}/scripts/autospec-startup-self-update.sh" \
    "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-startup-self-update.sh"; do
    if [ -f "$_candidate" ]; then
        AUTOSPEC_SELF_UPDATE_SCRIPT="$_candidate"
        break
    fi
done
if [ -n "$AUTOSPEC_SELF_UPDATE_SCRIPT" ]; then
    bash "$AUTOSPEC_SELF_UPDATE_SCRIPT" "$SKILL_NAME"
else
    echo "WARN: self-update skipped (autospec-startup-self-update.sh not found); continuing on installed version" >&2
fi
```
