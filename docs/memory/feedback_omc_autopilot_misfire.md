---
name: OMC autopilot magic-keyword false-trigger from system reminders
description: OMC's autopilot skill auto-activates when the literal word "AUTOPILOT" appears in a system-reminder hook payload — even though the user never invoked /autopilot. Recovery requires state_write(active=false) + state_clear(skill-active).
type: feedback
originSessionId: d5e49fd5-4555-4cec-a538-54e67df0904a
---
The oh-my-claudecode plugin's `autopilot` skill has a magic-keyword
detector that fires on the literal token "AUTOPILOT" appearing in
*any* prompt-submit context — including when the harness injects a
system-reminder hook whose body documents the autopilot skill
(e.g. UserPromptSubmit hook output that begins
`[MAGIC KEYWORD: AUTOPILOT]\n---\nname: autopilot...`). The detector
cannot distinguish "user invoked /autopilot" from "system reminder
mentioned the autopilot skill", so a `/autospec` (or any other) run
where a system reminder happens to embed the word AUTOPILOT silently
activates an autopilot session in `.omc/state/sessions/<sid>/autopilot-state.json`
with `awaiting_confirmation: true`. The autopilot stop hook then
fires after every conversation turn — `[AUTOPILOT - Phase: unknown]
Autopilot not complete. Continue working...` — until cancelled.

**Fingerprint:**
- `mcp__plugin_oh-my-claudecode_t__state_list_active` shows `autopilot`
  active even though the user invoked something else (e.g. `/autospec`).
- `mcp__plugin_oh-my-claudecode_t__state_read mode=autopilot` shows
  `original_prompt` matching the user's most recent message and
  `reinforcement_count` > 0, plus `awaiting_confirmation: true`.
- `.omc/state/autopilot-state.json` is *absent* from the legacy
  top-level path — state lives in
  `.omc/state/sessions/<session-id>/autopilot-state.json`. A bare
  `ls .omc/state/` will miss it.

**Why:** the same session that produced this memory hit it
mid-`/autospec`, after a system reminder injected the autopilot
skill body via a UserPromptSubmit hook. The user never typed
"/autopilot" and was correctly annoyed at being looped on stop-hook
spam.

**How to apply:**

1. **Recognize early.** If the AUTOPILOT stop hook fires unexpectedly
   during a non-autopilot session, do NOT panic and do NOT run
   `/oh-my-claudecode:cancel` blindly. First confirm the misfire via
   `state_list_active` + `state_read mode=autopilot`.
2. **Recover non-destructively.** Mark autopilot inactive while
   preserving any genuine resume data:
   ```
   state_write(mode=autopilot, session_id=<sid>, active=false,
               state={reason: "misfire — magic keyword from system reminder, not user invocation", ...})
   ```
   Then clear the skill-active reinforcement state (mandatory final
   step from the cancel skill):
   ```
   state_clear(mode=skill-active, session_id=<sid>)
   ```
3. **Do NOT delete the autopilot state file directly via bash** — the
   cancel skill explicitly warns "Do NOT use this fallback for
   autopilot. Autopilot requires state_write(active=false) to preserve
   resume data." Use the MCP state tools.
4. **Tools are deferred.** The state tools
   (`mcp__plugin_oh-my-claudecode_t__state_*`) are deferred per the
   harness — load them via
   `ToolSearch(query="select:mcp__plugin_oh-my-claudecode_t__state_clear,...state_read,...state_write,...state_list_active")`
   before calling.
5. **Continue the original work.** Your real task (e.g. `/autospec`
   Phase 4 monitor) is unaffected by the autopilot state — it lives
   in a different track. Do not abort what you were actually doing.
