---
name: autospec-listen
description: |
  Use when the user mentions filing an issue or writing a spec mid-conversation. Trigger keywords are "file an issue", "new issue", "open an issue", "create a ticket", "write a spec", "design spec", "new spec", "start a spec". On an "issue" trigger, drafts a body from the last ~10 conversation turns and asks the user to approve before running gh issue create. On a "spec" trigger, hands off to /autospec-define. Lives at github.com/berlinguyinca/autospec/skills/autospec-listen.
---

# autospec-listen (harness-neutral)

Conversation listener that watches for issue-filing or spec-writing intent
mid-conversation and dispatches to the right downstream flow without forcing
the user to remember a slash command.

## Trigger flow

(Trigger-flow body is implemented in follow-up issues #44 and #45.)
