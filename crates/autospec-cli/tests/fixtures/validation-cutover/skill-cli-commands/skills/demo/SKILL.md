# demo

The autospec suite and an autospec design/spec PR are prose, not invocations.

```bash
autospec validate
PLAN_JSON=$("${AUTOSPEC_BIN:-autospec}" queue ready --repo OWNER/REPO)
if "$AUTOSPEC_BIN" repair-loop --help >/dev/null 2>&1; then :; fi
```

```json
{"note": "a json block is never a command surface", "autospec": "portfolio"}
```
