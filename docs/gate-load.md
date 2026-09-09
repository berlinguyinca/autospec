# Gate machine-load observability (#3963)

A gate run that claims an environmental precondition — "serial on an
otherwise idle machine", "no other test process running" — must **assert the
precondition and record what it observed**. The observation belongs in the
result alongside the numbers. A run that claims exclusivity **acquires it, or
reports that it could not** and labels the result accordingly. Long-running
background work **registers itself where a later run will look**.

The failure this guards against: an experiment claimed "serial on an otherwise
idle machine" while a 21-hour conversion loop ran `cargo test` in its own
worktree for the whole session. The "serial" arm had one competing test
binary, the "concurrent" arm had nine, and the difference was reported as if
it were the variable under test. What *was* checked — that the process the run
itself had started had finished — is not "nothing is running".

## Primitives (`autospec-core::execution::gate_load`)

| Item | Job |
|---|---|
| `parse_process_lines` | Parse a `pgrep -af`-style listing (`<pid> <command...>` per line). Fails closed on a non-numeric pid: an unreadable observation is not "no process". |
| `is_test_process` | A command is a competing *test* process when `test` appears as a whole argv token (`cargo +1.91.0 test -p …`). A competing build is load, not a competing test binary; substring matches (`latest`) are not test processes. |
| `MachineLoad::observe` | The run's own pid and its ancestors never count against the run; everything else running a test is contention by definition. |
| `MachineLoad::line` | The recorded observation: `machine idle: no competing test process observed` — idleness reported as *observed*, never assumed — or `machine contended: N competing test process(es): <pid> <command>; …`. |
| `acquire_lease` | The named lease is a directory with an `owner` file (created `O_EXCL`). A live holder → `Contended { holder }`, never stolen. A dead holder's stale lease is reclaimed once; a lease that still cannot be taken fails closed. |
| `LeaseAcquisition::line` | The exclusivity line recorded in the result, for either outcome: `exclusivity: held lease …` or `exclusivity: NOT held — lease … held by <pid> <command>; this result ran under contention`. |
| `register_job` / `unregister_job` / `live_jobs` | The job registry: long-running background work writes a named record (name, pid, command, started) where a later run will look; `live_jobs` lists live holders and prunes dead ones — a dead pid is not running, and its record would lie. An unreadable record is an error, not absence. |
| `GateRunRecord` | The recorded run: result line + observed load + exclusivity, on one line. `claims_idle_machine()` is true only when the observation says idle and no claimed lease was contended — a "clean" result observed under contention does not get to claim a clean serial condition. |

## What a gate run owes the record

1. Observe the machine (`pgrep -af cargo` or equivalent) and record the load.
2. If it claims exclusivity, take the lease or record the holder.
3. Record result, observation, and exclusivity together; the rendered line
   must let a reader see a contradiction between "clean" and "contended" on
   one line.
