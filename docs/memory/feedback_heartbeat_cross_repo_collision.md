---
name: feedback-heartbeat-cross-repo-collision
description: "Process-heartbeat directory at ~/.autospec/process-heartbeats/ is shared across repos — stale heartbeats from sibling autospec sessions bleed into the local monitor's view"
metadata: 
  node_type: memory
  tags: 
    - autospec-run
    - cross-session
    - shared-state
    - watchdog
  type: feedback
  wing: synthesis
  drawer_class: lesson
  originSessionId: 2d7883e9-9977-428f-8919-ef9b88df12a4
---

**Discovered 2026-05-22:** while checking monitor state for autospec docs-amendment final batch, found stale heartbeat `~/.autospec/process-heartbeats/8.json` referencing `codingsandmore/vacuum-clamping-system` repo (issue #8, branch `feat/model-metadata-release-gate`, PR #19). The local autospec repo only has issues #319+, so `8` is impossible there.

**Why:** the heartbeat directory is one shared location `~/.autospec/process-heartbeats/` keyed only by issue number. Different repos with overlapping issue numbers will collide. The watchdog reconciler trusts the heartbeats it finds without verifying the `repo` field matches the current cwd's origin.

**How to apply:**
1. **Don't trust raw `ls ~/.autospec/process-heartbeats/`** for "what's in flight in MY repo?" — always filter by reading each JSON and matching the `repo` field against `gh repo view --json nameWithOwner` for the current dir.
2. **Watchdog reconciler should grow a repo-filter pass** — any heartbeat whose `repo` doesn't match the local origin should be ignored (not deleted, since the other session owns it).
3. **Heartbeat path itself should be repo-scoped** — better: `~/.autospec/process-heartbeats/<repo-slug>/<issue>.json`. Eliminates collisions entirely. File as a follow-up improvement.

**Why this matters now:** when looking at monitor state across crashes/recoveries, I've been treating any heartbeat in the directory as belonging to autospec. Cross-repo bleed can lead to false-positive "issue in progress" signals + wasted recovery actions on issues that aren't even in this repo.
