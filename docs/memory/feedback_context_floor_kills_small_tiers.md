---
name: Size local context windows from the measured session floor, not round numbers
description: The context present before any work (system prompt, memory, MCP/skill schemas) is 14.5k median but 37.9k at p90 for OpenCode; a 32k tier cannot start a heavy session
type: feedback
wing: synthesis
drawer_class: lesson
---
When sizing a local model's context window, three numbers decide it, and
`llm/linux-qwen38/scripts/analyze-session-contexts.py` measures all three
from real sessions (Claude Code `.jsonl` transcripts and the OpenCode
SQLite database):

- **floor** — context carried before any work happens: system prompt,
  project instructions, memory, skill and MCP tool schemas
- **growth** — tokens added per turn, measured on the *rising* segments
  only (a transcript that compacts is a sawtooth; averaging across the
  drops understates the fill rate)
- **coverage** — what fraction of turns fit, and what fraction of
  sessions never need to compact

Measured on this operator, 2026-08-18:

| | OpenCode | Claude Code |
|---|---:|---:|
| floor p50 | 14,492 | 39,655 |
| floor p90 | 37,873 | — |
| floor max | 76,006 | 70,272 |
| growth/turn | 764 | 1,312 |

**Why:** the floor is what makes small tiers unusable, and it is easy to
miss because it is invisible in "how long are my conversations". A 32k
tier looks fine and cannot start a p90 OpenCode session — the window is
full before the first question. The floor is also **client-specific**:
Claude Code's is nearly 3x OpenCode's because the system prompt and
skill set are larger, so a number measured on one client must not be
carried to another.

**How to apply:** require `tier >= p90_floor + (desired_turns x growth)`.
For 30 productive turns on OpenCode: `37,873 + 30 x 764 ~= 60,800`, so
40k is tight, 64-80k is comfortable, and <=32k is a trap. Coverage says
where to spend: 40k -> 80k moved "sessions that never compact" from
28.6% to 69.6%, while 80k -> 160k added only 20 points and cost the
ability to run a second session at all.

Housekeeping that follows: compaction is a **full re-prefill** (50-115s
at large contexts) because the cached prefix changes, so prefer fewer
large compactions; trimming unused MCP servers and skills lowers the
floor, which is paid on every turn and buys nothing; and prefer a fresh
session over a compacted one when the task changes.

Related: [[feedback_shared_kv_pool_has_no_admission_control]] — the tier
sizes also have to fit the shared pool, so floor and pool constrain each
other on a 24 GiB card.
