# autospec flag-file reference

autospec reads small **sentinel files** under `~/.autospec/` (the state dir; override with `AUTOSPEC_STATE_DIR`) to toggle behavior without env vars or arguments. A flag is "set" when the file **exists** (contents are ignored). Create one with `touch`, clear it with `rm`.

```bash
touch ~/.autospec/no-secaudit.flag     # set
rm    ~/.autospec/no-secaudit.flag     # clear
```

| Flag file | Effect | Set / cleared by |
|---|---|---|
| `stop.flag` | The autospec-run monitor halts after the current issue finishes. | `/autospec-stop --graceful` (set); `/autospec-stop --resume` (clear) |
| `autonomous.flag` | Marks the run as operating in autonomous mode (affects confirmation prompts). | autospec autonomous entrypoints |
| `init-done.flag` | Records that first-run init/bootstrap completed; skips re-init. | autospec init / sweep wizard |
| `no-review.flag` | Skips the end-of-run Phase 5.5 gap-remediation broad review. | operator |
| `no-secaudit.flag` | Skips the post-batch security sweep dimension (Phase 5.5). | operator |
| `no-test.flag` | Skips the test step (use only when tests are run elsewhere). | operator |
| `no-heal.flag` | Disables the autospec self-heal loop. | operator |
| `qa-no-heal.flag` | Disables the autospec-qa self-heal loop specifically. | operator |
| `qa-heal-stop.flag` | Stops an in-progress autospec-qa heal loop after the current round. | `/autospec-qa` stop path |
| `refine-loop-stop.flag` | Stops an in-progress `/autospec-refine --continue` loop. | `/autospec-refine` stop path |
| `continue-no-loop.flag` | Makes `/autospec-continue` run once instead of looping. | operator |
| `explore-stop.flag` | Stops the `/autospec-explore` perpetual loop after the current round. | `/autospec-explore` stop path |

Notes:
- The monitor and loops check these at round/step boundaries, so a flag takes effect at the next safe checkpoint, not mid-step.
- `stop.flag` is the global halt; the `*-stop.flag` variants halt one specific loop without stopping everything else.
- Removing a flag re-enables the behavior on the next run; no restart of unrelated work is needed.
