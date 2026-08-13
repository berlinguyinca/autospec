## Startup self-update

```bash
SKILL_NAME=canonical
FAILURE_RECORD="$HOME/.autospec/last-update-failure.json"
UPDATE_LOG="$HOME/.autospec/self-update.log"
REMOTE_VERSION="$HOME/.autospec/remote-version"
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh
bootstrap --skill all --harness all --update
if [ "$REMOTE" = "$LOCAL" ]; then date > "$LAST.tmp"; fi
tail -c 65536 > "$UPDATE_LOG"
jq -n --argjson installer_exit_code "$RC" > "$FAILURE_RECORD"
```
