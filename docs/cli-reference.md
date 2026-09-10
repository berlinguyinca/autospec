# CLI Reference

The Rust CLI is additive. Runtime environment management is implemented by the
Rust command family below; existing `/autospec-*` skills and unrelated shell
scripts remain operational surfaces while V62+ commands mature.

| Command | JSON | Status |
| --- | --- | --- |
| `autospec init --spec <id> [--spec <id>]... [--json]` | yes | initialize persisted planned state without executing work; refuses existing state |
| `autospec handoff probe --repo OWNER/NAME --intent TEXT [--artifact issue:N|spec:PATH] [--correlation ID] [--intent-kind implement|explain|plan] [--repo-dir PATH]` | yes | side-effect-free producer for the autospec.implementation-handoff.v1 handoff: probes the installed workflow surface and local run-state, then returns one typed route state (run/start/split_then_run/recover/none) or a typed unavailable reason with zero mutations |
| `autospec handoff capabilities` | yes | advertise the installed handoff workflow surface (autospec.handoff-capabilities.v1) |
| `autospec doctor --json` | yes | implemented |
| `autospec aar classify --title <text> [--body <text>\|--body-file <path>] [--label <l>]... [--path <p>]... [--files <n>] [--language <name>] [--json]` | yes | deterministic task classification with evidence and confidence; no LLM call |
| `autospec aar plan --title <text> [...] [--policy-version <v>] [--json]` | yes | full execution policy: topology, model, reasoning budget, retrieval ladder, guards, escalation |
| `autospec aar explain --title <text> [...]` | no | prose explanation of the selected profile and why |
| `autospec aar memory init [--worktree <dir>] [--json]` | yes | scaffolds `.autospec/` durable task memory; never overwrites existing state |
| `autospec aar rules` | no | prints the harness working rules injected into every agent session |
| `autospec doctor --readiness --json` | yes | implemented target-repo readiness report |
| `autospec doctor code-intel [--json]` | yes | code intelligence backend, language-server and fallback health ([docs](code-intelligence.md)) |
| `autospec doctor failures [--runs-dir <dir>] [--last <n>] [--threshold-percent <n>] [--top <k>] [--json]` | yes | failure signatures over a rolling window of fleet agent runs: counts against the window denominator, `SYSTEMIC` above the repetition threshold (exit `1`), `<no output>` / `<no status file>` counted as their own buckets ([docs](failure-signatures.md)) |
| `autospec cost [--out-dir DIR] [--since ISO-8601] [--threshold-percent N] [--json]` | yes | GPU-hours by terminal status over `out/issue-*/status.txt` run records: runs, GPU-hours, and share per status (window + cumulative), rework (discarded/held/superseded) separated from productive hours, known-defect costs, and flags for buckets above the share threshold (default 10%) |
| `autospec status [--json] [--limit <n>]` | yes | persisted local spec-lifecycle counts; `--limit n` caps list lines in the text rendering and ends with a `… K more lines (T total)` notice on stdout when the list was truncated |
| `autospec plan [--input <package-dir>] [--json]` | yes | read-only inspection of generated spec metadata |
| `autospec initiative init --id INIT-YYYY-NNNN --slug <slug> [--spec <path>] [--root <dir>]` | yes | creates the Initiative artifact registry and its first audit event; refuses an existing Initiative |
| `autospec initiative validate --id INIT-YYYY-NNNN [--json]` | yes | checks the Definition, workspace, plan, and task DAG against each other; exits `1` when the Initiative is not executable |
| `autospec initiative ready --id INIT-YYYY-NNNN [--now <unix>] [--json]` | yes | read-only scheduler pass; lists releasable tasks and the first reason each blocked task is held |
| `autospec initiative coverage --id INIT-YYYY-NNNN [--json]` | yes | requirement coverage matrix, including evidence discarded for lacking independence |
| `autospec initiative verify --id INIT-YYYY-NNNN [--json]` | yes | final completion gate; exits `1` while an unwaived requirement is unverified |
| `autospec initiative project --id INIT-YYYY-NNNN [--json]` | yes | renders the GitHub projection from canonical state and stores it; performs no GitHub mutation |
| `autospec initiative status --id INIT-YYYY-NNNN [--json]` | yes | Initiative snapshot: stage, repository and owner span, task states, requirement coverage, completion |
| `autospec validate [--path <changed-path>]... [--json]` | yes | read-only affected-check planner; shell wrapper remains the executor |
| `autospec validate --shadow-results <captured-results.json> [--json]` | yes | aggregates captured shell outcomes without executing commands; returns non-zero when a required captured result failed |
| `autospec runtime classify <path> --json` | yes | implemented R0-R4 ownership classification for one repository path |
| `autospec runtime audit --json` | yes | implemented read-only R0-R4 inventory; it neither migrates nor executes candidates |
| `autospec runtime env init [--repo <path>] [--manifest agent\|autospec] [--force]` | no | creates a conservative v1 runtime manifest; refuses an existing manifest without `--force` |
| `autospec runtime env up [--repo <path>] [--mode <mode>]` | no | provisions or reuses the selected environment, runs its manifest command on first provision, and prints the sourceable environment protocol |
| `autospec runtime env status [--repo <path>] [--mode <mode>]` | no | prints a provisioned environment or returns status `3` when it is inactive |
| `autospec runtime env down [--repo <path>] [--mode <mode>] [--purge-maven]` | no | removes owned Compose resources; `down --purge-maven` also removes the guarded Maven 4 environment prefix |
| `autospec runtime env exec [--repo <path>] [--mode <mode>] -- <command> [args...]` | no | provisions or reuses state, then runs one direct child with the runtime environment |
| `autospec runtime env session [--repo <path>] [--mode <mode>] [--keep-alive] -- <command> [args...]` | no | runs one direct child with lifecycle cleanup, manifest auto-init/bypass controls, and Unix interruption cleanup |
| `autospec runtime env gc [--repo <path>] [--mode <mode>]` | no | removes only stale resources whose generation and ownership labels are proven; ambiguity fails closed with a recovery command |
| `autospec runtime env normalize-compose --repo <path> --check\|--apply [--fingerprint SHA256]` | yes | plans or transactionally applies a manifest-v2 Compose migration without a second YAML transformer |
| `autospec claim state read\|upsert\|clear\|reconcile-linked-pr ...` | yes | manages the schema-1 GitHub run-state comment using lowest-comment-ID selection |
| `autospec claim acquire\|release ...` | yes | applies the typed safety gate, heartbeat/label ordering, lease CAS, and terminal release transitions |
| `autospec claim branch-live <BRANCH> [OWNER/REPO]` | yes | the single definition of branch-attempt liveness (open or merged PR, or checked out in a local worktree; a branch whose only PR is closed-unmerged is abandoned), printing one JSON verdict with the deciding reason (`open-pr`, `merged-pr`, `worktree-checked-out`, `branch-missing`, `no-open-or-merged-pr`); exit `0` live, `1` not live, `2` on error — shell tools call this instead of re-deriving the rule (#4146) |
| `autospec issue promote --repo OWNER/REPO --number N [--remove-label needs-autospec-template]` | yes | validates the canonical GitHub issue, records review with owned labels without editing its body, and verifies authoritative re-reads |
| `autospec queue ready [--repo OWNER/REPO] [--batch-size N]` | yes | scans every Rust-owned GitHub issue page and returns typed eligibility, gate totals, and scan scope |
| `autospec queue review-safety --repo OWNER/REPO --limit N [--issue N]` | yes | writes bounded Rust issue-intent safety decisions and reports outcome totals |
| `autospec resources list [--run <run-id>] [--type <resource-type>] [--json]` | yes | reads the persistent resource ledger (spec §24.4) and prints one row per resource — id, type, state, ownership, run id; `--json` emits a JSON array; exits `0` on an empty ledger |
| `autospec resources show <resource-id> [--json]` | yes | prints one ledger record by id (a JSON object with `--json`); exits non-zero when the id is absent; no subcommand deletes or mutates a row |
| `autospec graph analyze --input <PATH> [--capacity N] [--format json\|text]` | yes | analyzes a proposed issue DAG (issue graph JSON or an issue draft array): §19 metrics JSON in stable key order with `estimated_fleet_saturation` and the 12 `autospec.define` telemetry counters, or the §24 wave projection plus §25 planner summary with `--format text`; exits `3` with the 1-line cycle path on a cyclic graph |
| `autospec autonomous resilience decide --repo OWNER/REPO [--issue N] [--budget-tokens N] [--budget-issues N]` | yes | reads resilient admission state without migration; atomic lifecycle ownership writes only canonical `owner__repo` state and starts no shell process |
| `autospec autonomous drain --repo OWNER/REPO --repo-dir DIR [--stall-secs N] [--poll-secs N]` | yes | directly supervises the fixed `omx exec ... $autospec-run` child, preserving local/external progress and terminating only a genuinely stalled live child |
| `autospec autonomous blast-radius --changed-files FILE [--fenced-surfaces YML] [--json]` | yes | classifies changed paths against configured fenced surfaces; fenced matches exit non-zero and report quarantine evidence |
| `autospec autonomous main-health --repo OWNER/REPO --repo-dir DIR [--branch BRANCH] [--json]` | yes | runs the Rust repository-local mainline-health probe without dispatching work |
| `autospec autonomous start --repo OWNER/REPO --repo-dir DIR [--epic N] [--max-cycles N] [--poll-interval-sec N]` | no | refreshes to the checkout's exact immutable runtime, creates or adopts one verified run epic, then launches a lease-owned conductor |
| `autospec autonomous resume --epic N --repo OWNER/REPO --repo-dir DIR` | no | verifies and reconstructs a managed accountability epic, reopening a closed or parked epic when safe, before conductor spawn |
| `autospec autonomous run-foreground --repo OWNER/REPO --repo-dir DIR [--branch BRANCH]` | no | runs one native foreground cycle when invoked directly; a child launched by `start` inherits lifecycle ownership and repeats cycles |
| `autospec autonomous lifecycle decide --repo OWNER/REPO [--claim-repo OWNER/REPO --claim-issue N --claim-worker ID --claim-branch NAME --claim-state active\|terminal] [--lease-age-sec N] [--stop graceful\|immediate] [--health continue\|wait\|halt] [--budget within\|soft\|hard] [--ready-tier 1\|1.5\|2\|3\|4\|5\|6\|7\|idle]` | yes | evaluates one pure typed lifecycle decision without filesystem, process, GitHub, shell, or `omx` effects |
| `autospec autonomous executor-result --repo OWNER/REPO --issue N [--worker-id ID --branch NAME --outcome succeeded\|blocked\|retryable ...]` | yes | records either the exact legacy deferred receipt or one strictly validated executor outcome; it never launches work, releases a claim, or merges a PR |
| `autospec run --run <id> --spec <id>[=<source.md>]... [--json]` | yes | creates a local persisted queue only; it does not launch an agent or validation command. Every `--spec` value is a bare spec id, or a spec id plus a source file staged in the same action (`id=<source.md>` — staged inputs land at `.autospec/runs/<id>/specs/<spec-id>.md` before the queue is written, and an unreadable source aborts the creation fail-closed); either all `--spec` values carry a source path or none |
| `autospec run --ingest <agent-result.json> --run <id> --spec <id> --result-id <id> --outcome <passed\|failed\|blocked\|no-spec> [--failure-kind <kind>] [--retry-limit <n>] [--json]` | yes | validates and records an explicit local agent result; it does not launch an agent or validation command. `--outcome no-spec` parks the entry in the terminal `no-spec` state ("dispatched but could not start") with a required blocker, no failure kind, and no attempts increment — it is excluded from the re-dispatch selector and must not be reported as `no-output` |
| `autospec resume [--json]` | yes | reports the newest incomplete local queue and its next entry; it does not execute it |
| `autospec report --json` | yes | local release summary from persisted spec state |
| `autospec rag config [--set KEY=VALUE] [--json]` | yes | renders the effective `agentic_rag:` configuration and rejects an invalid one (a revision-blind cache, an unknown key) |
| `autospec rag policy [--role ROLE] [--json]` | yes | prints per-role source ordering, context ceiling, sufficiency threshold, and whether the role must verify independently |
| `autospec rag sources [--role ROLE] [--external] [--json]` | yes | reports, per source, the administrator's availability setting and whether that role and task may actually reach it |
| `autospec rag route --task TASK [--context N] [--node id:reasoning:free_context:speed:seats]... [--json]` | yes | explains one InferWeave routing decision: required context including margin, the selected node, whether it is a `saturated_fallback` (selected with no free seats, to be queued), and why each other node was rejected |
| `autospec evaluator init [--policy <file.json>]` | yes | writes `.autospec/evaluation/policy.json` (the default policy when omitted), `epoch-000000`, and `current.json`; re-init fails |
| `autospec evaluator register --file <definition.json>` | yes | validates a definition, writes immutable `evaluators/<slot>/v<N>.json`, and prints the definition digest |
| `autospec evaluator list` | yes | `slot@version kind digest[..16] created_at`, one per line |
| `autospec evaluator show <slot@version>` | yes | pretty JSON plus digest |
| `autospec evaluator pin <slot@version> --actor <name>` | yes | seeds an empty slot and creates the next epoch |
| `autospec evaluator epoch current` | yes | epoch id, started_at, policy digest, one line per slot |
| `autospec evaluator epoch history` | yes | every epoch oldest-first with its promotion id |
| `autospec evaluator challenger run --incumbent <slot@v> --challenger <slot@v> --suite <suite-id@v> --incumbent-verdicts <file.json> --challenger-verdicts <file.json>` | yes | pairs verdicts, qualifies against the promotion policy, writes `trials/<id>.json`, and prints the report and trial id |
| `autospec evaluator challenger inspect <trial-id>` | yes | pretty JSON of the trial |
| `autospec evaluator promote <trial-id> [--approve --actor <name>]` | yes | `Approval::Human` with `--approve --actor`, else `Approval::Policy`; fail closed on missing qualification or approval |
| `autospec evaluator record add --file <record.json>` | yes | write-once evaluation record |
| `autospec evaluator record list [--active \| --stale]` | yes | `id evaluator epoch outcome status`, one per line |
| `autospec anchor register --file <suite.json>` | yes | validates the suite, verifies artifact digests, writes it once, and appends the journal |
| `autospec anchor list` | yes | `id@version slot cases digest[..16]` |
| `autospec anchor show <suite-id@version> [--role operator\|qualification\|mutation]` | yes | default `mutation` (protected-holdout labels redacted) |
| `autospec anchor verify <suite-id@version>` | yes | recomputes artifact digests; exit 2 naming mismatched cases |
| `autospec showcase --json` | yes | demo stub |
| `autospec benchmark validate-matrix <matrix.json>` | no | validates a Qwen3.8 benchmark matrix against the provider-neutral contract (#3328): every row identifies quantization, runtime, node, profile; speculative rows record `draft_tokens`/`accepted_draft_tokens`; a candidate wins only when `success: true`; only supported local cells run (Q3-Q8, supported runtimes/nodes) and skip reasons are preserved; the report compares the winner's median successful issue time to the baseline |
| `autospec benchmark validate-matrix <matrix.json> --json` | yes | machine-readable form: `valid`, per-row `errors`, per-cell `cells` (run/skip + reason), and the `report` |
| `autospec benchmark` | no | any other invocation exits non-zero with usage |
| `autospec growth-report --json` | yes | local-only metrics stub |
| `autospec repair-loop record --loop <name> [--expected <id>]... [--repaired <id>]... [--ticket <id=ticket>]... [--state-file <path>]` | no | records one repair sweep; exit 0 idle / 1 repaired / 2 persistent (ALERT) |
| `autospec repair-loop status --loop <name> [--state-file <path>] [--json]` | yes | ledger summary: repair rate over the rolling window, active per-identity streaks, attached defect tickets |
| `autospec dispatch check [--queue <path>] [--state-file <path>] [--topology <path>] [--admitted-file <path>] [--now <epoch>] [--interval <secs>] [--max-intervals <n>] [--json]` | yes | gate on the queue artifact before dispatching: exit 0 fresh (proceed or genuinely idle) / 1 hold (missing, unstamped, stale, clock rewind); with `--admitted-file`, an idle queue over admitted work holds as `ADMITTED_NOT_SCHEDULABLE` instead of reading idle (#3927) |
| `autospec dispatch reconcile --admitted-file <path> [--queue <path>] [--json]` | no | the admission reconciliation (#3927): report the count of admitted-but-unschedulable issues; exit 0 zero (the expected answer) / 1 nonzero (a defect — the queue lags the tracker) |
| `autospec dispatch guard --issue <N> [--out-dir <path>] [--patch-name <name>] [--dry-run] [--json]` | yes | pre-dispatch gate on the issue's output directory (#3764): exit 0 no unconverted patch (stale output removed) / 1 hold (patch present, or a check that cannot answer); `--dry-run` reports without touching the directory |
| `autospec dispatch stage --issue <N> [--repo OWNER/REPO] [--issue-json <path>] [--comments-json <path>] [--body-file <path>] [--title <t>] [--source-updated-at <ts>] [--body-updated-at <ts>] [--out <path>] [--staged-at <epoch>] [--container-runtime <path>] [--database <value>] [--registry <value>] [--gate NAME=COMMAND]... [--no-probe] [--json]` | no | write the spec a worker reads: issue body, the discussion filed since the last body edit, a generated execution-environment block, and — when `--gate` flags are passed — the gate set the patch is graded against, rendered as acceptance criteria so grading can never run a weaker set than the spec names (#3925); headed by the source `updatedAt` it was staged from (#3864); an empty issue body is refused (a spec without its task text is `NO-SPEC` and no dispatch would start on it, #3620); the output names the receipt (`spec-bytes` / sha256) of what it wrote; defaults to `~/.autospec/dispatch/specs/<N>.md`, exit 2 on unusable input (including an empty issue body, or a malformed or duplicated `--gate`) |
| `autospec dispatch freshness --issue <N> [--staged <path>] [--live-updated-at <ts> \| --live-json <path> \| --repo OWNER/REPO] [--status-file <path>] [--json]` | yes | gate a dispatch on the staged spec's revision (#3864): exit 0 current / 1 held — `STALE` (re-stage, the message names the command) or `REFUSED` (no staged spec, an **empty staged spec, which reads `NO-SPEC`** (#3620), no recorded revision, or the live issue cannot be read: freshness that cannot be verified is never verified); with `--status-file`, the spec receipt (`spec-bytes` + `spec-sha256`) is appended to the run's `status.txt` |
| `autospec dispatch preflight --issue <N> [--staged <path>] [--prompt-file <path>] [--live-updated-at <ts> \| --live-json <path> \| --repo OWNER/REPO] [--status-file <path>] [--json]` | yes | the single pre-dispatch gate — staging is part of dispatch, not a separate manual step a caller can forget (#3620): refuses `NO-SPEC` before a single token is spent when the staged file is missing or empty, asserts the assembled `--prompt-file` carries the spec's text (a prompt whose issue section is empty is a programming error), then runs the freshness gate; exit 0 dispatch authorized / 1 held (`NO-SPEC`, prompt refusal, `STALE`, or `REFUSED` — the line names which check stopped it) / 2 caller fault (unreadable prompt file) |
| `autospec dispatch stamp [--by <name>] [--queue <path>] [--state-file <path>] [--at <epoch>]` | no | the producer's call: writes `# refreshed-at:` / `# refreshed-by:` atomically and beats for its own hop |
| `autospec dispatch beat --step <name> [--state-file <path>] [--at <epoch>] [--json]` | yes | one liveness stamp for one hop; the ledger is monotonic, an older beat is ignored |
| `autospec dispatch status [--topology <path>] [--state-file <path>] [--now <epoch>] [--interval <secs>] [--max-intervals <n>] [--json]` | yes | declared topology, credential-holding steps and their hosts, per-hop verdicts, static topology audit; exit 0 healthy / 1 any defect |
| `autospec dispatch runs --runs <path> [--out <path>] [--duration-floor <secs>] [--quote-bytes <n>] [--fault-threshold <n>] [--json]` | yes | classify a dispatch batch (#3918): each run is `OK` / `NO-OUTPUT` / `INFRA-FAIL` (auth, endpoint, context — never consumes an attempt) / `LAUNCH-FAIL` (under the duration floor regardless of transcript); writes the `agent-status.tsv` record (transcripts at or below the quote threshold ride along verbatim), prints the batch summary plus a `FLEET-FAULT` line for any repeated identical failure and `SUBFLEET-IDLE` lines for sub-fleets with zero agents but open eligible work; exit 0 / 1 fleet fault |
| `autospec dispatch tick [--queue <path>] [--state-file <path>] [--lifecycle <path>] [--topology <path>] [--now <epoch>] [--interval <secs>] [--max-intervals <n>] [--json]` | yes | one dispatch tick over the queue (#3911): the liveness gate first (a hold prints its `LIVENESS FAILURE` line and exits 1), then a per-entry report — fresh work is `dispatch`, produced-but-unconverted re-enters as `convert`, converted entries are skipped and named, held entries carry their hold reason; exit 0 something dispatched / 1 liveness hold or nothing dispatched with skips to name / 2 diagnostic |
| `autospec dispatch mark --action <produced\|converted\|hold\|release> --issue <N> [--reason <text>] [--at <epoch>] [--lifecycle <path>]` | no | move one queue entry through its lifecycle (#3911): `produced` (the agent produced a patch), `converted` (terminal), `hold` (record why it is blocked — `--reason` required — and preserve its state), `release` (clear the hold; a released produced entry re-enters the next tick as a convert, not a fresh dispatch); stamps are monotonic, so a backwards or terminal-entry stamp is refused with exit 1 and a usage error exits 2 |

`autospec repair-loop` observes a self-healing loop so that a repair which keeps
repairing the same identity reads as an alert, not a status line. `record` feeds one
sweep into a durable per-loop ledger (JSON under `~/.autospec/repair-loops/` or
`--state-file`) and prints a verdict line that states the repair **rate** over a
rolling 5-sweep window, names the **consecutive-sweep count** for any identity repaired
on 3 or more consecutive sweeps ("`w1 re-registered on 4 consecutive sweeps`"), and
prints healthy and unhealthy runs differently (`0 missing (expected 0)` versus
`4 missing` — never the same line). A persistent identity must trace to a defect
ticket (`--ticket <id=ticket>`); one without a ticket is printed as `UNTRACKED DEFECT`,
which is the case where the repair is standing in for an unhealed defect nobody is
reporting. Exit codes make the verdict machine-readable for cron/CI: `0` idle,
`1` repaired, `2` persistent.

`autospec dispatch` puts a liveness stamp on every hop between filing an issue and
dispatching an agent, so a queue that stopped being repopulated reads as an error instead
of as "nothing to do" (#3800). The queue artifact (`~/.autospec/queue.txt`) carries two
header lines written by its producer — `# refreshed-at: <epoch>` and
`# refreshed-by: <step>` — and `check` refuses to call an artifact with neither a
freshness stamp: `QUEUE_MISSING`, `QUEUE_UNSTAMPED`, `STAMP_NOT_REFRESHED` (older than
`--max-intervals` of the producer's own `--interval`), `CLOCK_REWIND` (a stamp in the
future). Exit codes are cron-shaped throughout: `0` ok, `1` hold, `2` diagnostic.
The queue is a *derived* copy of the tracker's label set, and the label set is
authoritative (#3927): an issue labelled dispatchable is admitted, and nothing else
gates it — there is no manual staging step between the label and schedulability.
`reconcile --admitted-file <path> --queue <path>` is the periodic check that the two
still agree: it reports the *count* of admitted-but-unschedulable issues (admitted by
the tracker but missing from the queue), and zero is the only expected answer — any
other number is a failure, because it is filed, label-bearing work the scheduler will
never reach. A queue entry the tracker no longer admits is reported as stale (clean up
the queue), never as the defect. `check --admitted-file <path>` folds the same set into
the dispatch gate: when the queue is fresh but empty *and* the admitted set is not, the
dispatcher holds with `ADMITTED_NOT_SCHEDULABLE` (naming the refresh step that must
repopulate the queue) instead of claiming idleness — an idle dispatcher over eligible,
unfiled work is a fault, not silence.
A fifth concern lives in the same chain, because it was born in it (#3961): two
independent symptoms both landed in the same dispatch-queue file and collided on
dispatch, and nothing at filing time compared them. Issues are filed by symptom and
implemented by file, so the comparison happens on the *file*, not the subject. Each
issue records its predicted write surface — the paths or modules its fix expects to
touch, read from the `## Files touched` section of its body — and filing checks that
surface mechanically against every open issue's, reporting each shared entry as
`WRITE-SURFACE OVERLAP` with the sibling named at filing time, because the fix there is
one sentence and after dispatch it costs a GPU run plus a merge a supervisor should not
be doing in someone else's interface. The same surfaces are the dispatcher's input to
serialisation: issues plan into concurrency waves, an issue joins the first wave its
surface is free in, and an issue with no declared surface takes a wave to itself — an
undeclared surface cannot be proven disjoint, so it never rides with neighbours. When a
sibling merges while an issue is still in flight, the re-staged spec carries a
`SIBLING LANDED` note naming what merged, what it introduced, and where, with the
instruction to extend rather than re-implement, because the re-dispatched agent's base
snapshot is the world before the sibling. The primitives are pure in
`autospec_core::dispatch_pipeline` (`IssueWriteSurface`, `FilingOverlapCheck`,
`DispatchWaves`, `SiblingLanding`).
The lifecycle of a queue entry is the sixth concern in the chain, because it was
born in it (#3911): a produced patch used to read as terminal — the entry left no
record of where it was, so a dispatcher that had to wait for it saw "nothing to
do" and the slot blocked forever. Entries now carry state in a consumer-owned
ledger (`~/.autospec/dispatch-lifecycle.json`, `--lifecycle`): `queued` (the
default, unknown entries), `produced` (mid-lifecycle, not terminal), `converted`
(the only terminal state), with a hold — a reason on top of any state — for an
entry that is blocked and must say why. `mark` moves one entry; stamps are
monotonic, a backwards stamp is refused, a hold on a converted entry is refused,
and `release` clears the hold so the entry re-enters the next tick as a
conversion, never as a fresh dispatch. `produced`-but-unconverted entries are
directly queryable — the ledger counts and lists them. `tick` is the dispatcher's
per-tick report: after the liveness gate, every queue entry is named — dispatched
fresh, converted, skipped (and why), or held (and why) — so a tick that dispatches
nothing still says what it is waiting on. The primitives are pure in
`autospec_core::dispatch_pipeline` (`LifecycleLedger`, `EntryState`,
`DispatchTick`).
`stamp` is what the refresh script calls after it repopulates the file: it rewrites the
headers through a temp file and rename, then records a beat for the producing hop, so a
script cannot refresh the artifact and forget to say so. `beat --step <name>` records
liveness for the other hops (`file-issue`, `topup`, `dispatch-agent`) into
`~/.autospec/dispatch-liveness.json`. `guard --issue <N>` is the pre-dispatch gate on the
issue's output directory (`<out-dir>/issue-<N>`, default `~/.autospec/dispatch/out`): the
dispatch path removes that directory before re-dispatching, and the guard verifies the
removal is safe by checking it holds no unconverted patch (`changes.patch` by default).
The check fails closed — a `stat` error is an unsafe answer, never a clear one, and a
missing check holds the dispatch — so the #3764 failure (an unanswerable existence check
read as "no patch", and the directory destroyed anyway) is now a `HELD` verdict. Without
`--dry-run`, an authorized guard removes the verified patch-free directory; a held guard
touches nothing. `status` reports the declared topology — the
built-in filing-to-dispatch chain unless `--topology` points at a JSON file with a
`{"steps": [...]}` array, whose `host` is one of `authenticated`, `shared-cluster`,
`ephemeral-session`, `credential` one of `none`, `gh-token`, and `schedule` either
`{"scheduled": {"interval_secs": N}}` or `"session-scoped"` — and audits it statically:
a credential-holding step scheduled on an `ephemeral-session` host, a credential held on
`shared-cluster` storage, a consumed artifact whose producer only runs from a login
session, or a step with no log are all reported as `TOPOLOGY DEFECT [CODE]` before a
single agent is launched. Host names and credential names in a hand-written topology file
are exactly the strings `status` prints.

`autospec dispatch stage` and `dispatch freshness` are the staged-spec gate (#3864). Staging
happens on the merge host, which holds the `gh` token; the cluster that runs the agent does
not, so a spec staged there is the only copy the worker will ever read — and a comment filed
an hour later is simply absent from it. `stage --issue <N>` writes that copy from the issue
payload plus the issue comments (`--issue-json`, `--comments-json`, or `--repo OWNER/REPO`,
which calls `gh api` on the staging host for the issue and, unless a
`--comments-json` file already supplies them, its comments — a discussion read that fails
there is a staging failure, never a spec that looks complete). Its leading header block records
`# source-updated-at:`, `# staged-at:` and `# comments-included: <kept> of <total>`. Only
comments after the last body edit are pulled in; when the payload carries no body-edit time
every comment is included and the spec says so, because including a comment the worker did
not need costs a paragraph and omitting one it needed costs the run. The `## Execution
environment` block states the container runtime, database and registry the staging host
probed and what was absent, so a worker told to `docker run` on a cluster that only has
Apptainer sees that before it fails. `freshness --issue <N>` is the gate: it compares the
recorded revision against the live one (`--live-updated-at`, `--live-json`, or `--repo`), and
prints one `STAGED-SPEC issue <N> current|STALE|REFUSED:` line. `STALE` is recoverable — the
line names the re-stage command to run. `REFUSED` is the case that used to be silent: no
staged spec, a spec staged before revision headers existed, or a live issue the cluster
cannot read. Freshness that cannot be verified is never verified, so a `gh` that returns
"not authenticated" on an unauthenticated cluster holds the dispatch instead of dispatching
yesterday's read.

`autospec rag` is read-only and performs no retrieval. It reports what the Agentic RAG
subsystem's configuration and policy *would* do, so an operator can check a role budget or a
routing rejection without running a retrieval or reaching a knowledge source. Retrieval itself
is a library API (`autospec_core::rag::RetrievalCoordinator`), not yet a command.

`autospec evaluator` and `autospec anchor` are the CLI surface for versioned evaluators and
frozen epochs ([architecture note](architecture.md#evaluator-epochs)). Every subcommand
accepts `--root <path>` (default `.`) and `--json`; diagnostics exit `2` as
`<kind>: <message>` where kind is one of `invariant`, `immutable`, `integrity`, `io`,
`parse`, `fail-closed`, or `access-denied`. Evaluator definitions, anchor suites, epochs,
trials, and records are write-once: a second write is an `immutable` failure naming the
path. State lives repo-local under `.autospec/evaluation/`, and the active epoch is the
single atomic pointer `current.json`. A challenger is promoted only at an explicit epoch
boundary, only when it qualifies on a labeled anchor suite under the policy at
`.autospec/evaluation/policy.json`, and only with `--approve --actor <name>` for slots
listed in `require_human_approval_slots`. `challenger run` prints, in order, the trial id,
one `incumbent:` and one `challenger:` line with `successes= failures= best_belief=`,
`margin_ppm: <n>  challenger_only_correct=<b> incumbent_only_correct=<c>`, one
`subset <name>:` line per required subset, then
`verdict: qualified | rejected (<reasons>) | inconclusive (<reasons>)`. `anchor show`
defaults to the redacted `mutation` role: protected-holdout cases carry no `expected_label`
and quarantine cases are dropped. `anchor verify` checks the *stored* suite, so quarantined
cases still have their artifacts digested and suite invariants are re-checked; the redacted
view is never validated, because a redacted label is legitimately absent rather than missing. See the [configuration reference](CONFIG_REFERENCE.md#evaluator-promotion-policy)
for the policy fields.

`autospec plan` only reads and parses Markdown from one generated package. It does not
execute validation, calculate an execution order, or report persisted lifecycle state.

`autospec validate --shadow-results` accepts the strict schema-1 captured-result shape used
by `crates/autospec-cli/tests/fixtures/validation-results/`: each row supplies a unique name,
Boolean `required`, and signed `exit_code`. Rust computes the pass/fail aggregate only; it never
spawns the captured command. The compatibility wrapper still delegates all real validation to
`autospec validate` until a full fixture-backed cutover is approved.

`autospec run` is deliberately a state-management command, not an execution engine. Queue
creation requires an explicit run ID and one or more spec IDs. Result ingestion requires a
strict `schemas/autospec-agent-result.schema.json` document plus an explicit outcome and
result ID; `failed` also requires `--failure-kind` (`validation`, `environment`, `agent`,
`dependency`, or `safety`). Results are retained append-only below
`.autospec/runs/<run-id>/agent-results/<spec-id>/<result-id>.json`, so a retry can safely
replay the same result ID without consuming another queue attempt. `resume` only reports the
current queue position. Use `/autospec-run` for the existing agent-execution workflow.

For the v1 runtime-manifest grammar, state behavior, child-command semantics, and cleanup
procedure, see [Agent runtime manifests](runbooks/agent-runtime-manifest.md).

Manifest `version: 2` adds typed Maven and Compose ownership. The opt-outs
`AUTOSPEC_MAVEN_ISOLATION=off`, `AUTOSPEC_COMPOSE_ISOLATION=off`, and
`AUTOSPEC_ENV_DISABLE=1` export `AUTOSPEC_ISOLATION_BYPASSED=1`; evidence produced under an
opt-out is not verified isolation. Unix state is private (`0700` directories, `0600` files),
and `RUNTIME_STATE_SYMLINK_REJECTED` prevents destructive cleanup through a linked root.

`autospec claim state` is the Rust-owned transport and codec for the existing GitHub
run-state comment protocol. `read` fails closed when the lowest marked comment is malformed
or bound to a different issue; `upsert` patches that lowest comment and removes higher-ID
duplicates; `clear` removes only marked run-state comments; and `reconcile-linked-pr` records
one eligible linked PR before posting the idempotent post-PR handoff reminder. `acquire` validates
the current issue safety review before it writes a startup heartbeat and moves labels, then uses
the lowest GitHub comment ID plus a server-side timestamp to decide the lease. `release` writes
terminal merge evidence before state and label transitions, and then retires the local evidence for
that claim: the issue heartbeat and the session binding whose recorded identity matches the released
claim. Retiring the session binding is what lets one worker session claim a second issue — the
binding is create-once, so a binding left naming a finished issue makes every later `acquire` from
that session fail with `heartbeat_write_failed`. Retirement is best effort and only runs when local
evidence exists; the remote record is already authoritative, and the next acquirer's predecessor
path retires anything left behind. When that predecessor retirement fails, the refusal reports the
predecessor's `claim_id`, which is the value `claim release --claim-id` needs. Legacy script
entrypoints remain only as compatibility surfaces until every caller is redirected to this command
family.

`autospec issue promote` owns the remote admission transaction. It fetches the canonical GitHub
issue, applies the repository's trusted-actor and regex policy, validates the existing canonical
safety section, records review with `safety:reviewed` without editing the body, re-reads the exact
state, and only then adds `auto-implement`. A final re-read detects concurrent title, body, author,
state, or label changes and rolls back transaction-owned labels. Completed admissions are
idempotent, and `--remove-label needs-autospec-template` lets the same transaction finish the
groomer's owned label transition with verified rollback on cleanup failure or drift.

The JSON response emits `"auto-implement": true` only for a passing verdict and reports
`eligible` for final-payload queue-policy eligibility plus `changed` for remote mutation. It
returns structured ambiguous/blocked/indeterminate decisions without admission and groups
blocked or indeterminate verdicts by inner safety reason in `blocked_by_reason`. `eligible` is
not a live claim, dependency, pull-request, worker-capacity, or path-conflict decision.

`autospec queue ready` follows every GitHub REST page for open `auto-implement` work and active
claims, counts raw issue-page records before filtering pull requests, and cursor-paginates linked
pull-request evidence while preserving check snapshots. A malformed or incomplete later evidence
page blocks selection rather than shortening the scan. Two open issues that cite the same
spec section and state the same goal are one task: the queue keeps the lowest-numbered issue
ready and blocks its twin with reason `duplicate_issue` and the canonical issue number in
`duplicate_of`, so a duplicate agent run is dropped with the drop visible instead of
surfacing later as a merge conflict. Its JSON includes a stable `gate_counts`
object for discovered, candidate, reviewed,
blocked, duplicate, dependency-blocked, linked-PR-blocked, path-conflicted, ready, claimed, and
selected issues. The `ready` list (and the `batch` drawn from it) is ordered by **unblocking value
descending**, ties broken by issue number ascending: a candidate's unblocking value is the count
of other open issues that transitively depend on it through `## Dependencies` edges, so a
pipeline-fixing foundation issue (dispatcher, gates, runner) is dispatched before the
lower-numbered leaf work it unblocks instead of after it. Selection of which issues are ready stays
in issue-number order; only the dispatch order of the already-ready set changes. A ready issue
that unblocks at least one other reports its count in a top-level `unblocks` field (omitted when
zero). `scan_scope` is `repository` for a full scan and `slice` when
`AUTOSPEC_RUN_ONLY_ISSUES` constrains the result, so callers cannot mistake a completed slice for
whole-queue completion.

`autospec queue review-safety --repo OWNER/REPO --limit N [--issue N]` requires a positive bound
and scans only unreviewed open `auto-implement` issues. `--issue N` re-reads and reviews that
exact admitted issue without scanning the queue, so groomers can safely bridge queue admission to
Rust-owned safety writeback. It emits
`pass`, `ambiguous`, `block`, `stale`, `conflicted`, and `skipped` totals. A pass writes one
canonical review block, adds `safety:reviewed`, and re-reads the issue through the typed claim
gate. Ambiguous issues receive `autospec:needs-human`; blocking issues receive
`security:quarantined`; neither becomes reviewed-eligible. Conflicting or malformed remote
evidence is fail-closed and counted as `conflicted`.

`autospec autonomous run-foreground` is the typed Rust control-plane and executor entrypoint.
One cycle performs bounded mainline-health admission and queue safety review, selects and claims
at most one ready issue, and persists strict conductor state as
`.autospec/autonomous-operator/<scope>/foreground-conductor-<scope-key>.json`, where the scope
key distinguishes repository runs from each explicit issue slice. For a selected issue it calls
the native executor bridge directly. The bridge resolves the configured Codex, Claude, or
OpenCode harness from the installed runtime-alias table, creates or adopts the exact private
issue worktree and runtime session, and launches the harness with an explicit local-only argument
vector. It never delegates implementation to a shell, `omx`, `/autospec-run`, or a second queue
owner.

Harness output and phase changes are appended to the repository-scoped autonomous log consumed
by `--follow`. Rust independently proves the resulting commit, Closeout report, runtime smoke,
full resolved suite, implementation and security scans, immutable premerge decision, required
CI, and LGTM review before it marks the draft ready or admin-merges. The harness cannot push,
create or edit a pull request, mutate claim state, or merge; those remote actions remain inside
the bridge and are rebound to the exact claim generation and head commit immediately before
mutation.

Bridge state is claim-generation scoped. A pending or active invocation remains non-terminal.
After a conductor restart, exact process identity permits observation of the existing supervisor
without launching a second harness; after process exit, the next run resumes from the last
durable phase. A private local acquisition receipt must match the authoritative repository,
issue, worker, branch, and claim ID before a restarted conductor adopts a live claim. Runtime
cleanup uses the invocation's persisted environment, session, and original manifest snapshot
even if the repository manifest later changes. Schema-1 snapshots from a pre-upgrade active
session reattach against the validated authoritative plan and are conservatively reported as
isolation-bypassed. Before clearing the acquisition receipt, the conductor persists an explicit
terminal- or ownership-retirement boundary so a crash cannot replay completed or lost work.
Transient GitHub reads after implementation retry the same claim generation and durable
invocation; a retryable terminal result preserves
recoverable committed or uncommitted work, advances it onto a changed base without force after
it becomes clean, and starts a fresh claim generation. Only an observed merged pull request,
explicit blocked result, or exhausted retry can become terminal, and no terminal result may
retain `in-progress-by-bot`.

Failure cleanup intent is persisted before runtime teardown. Ownership takeover closes only the
old exact runtime, records the worktree HEAD and status digest under the per-issue lock, and lets
the successor generation adopt the unchanged worktree. A pre-upgrade dispatch without a local
acquisition receipt is migrated only when an exact private invocation or terminal receipt proves
the authoritative claim; otherwise the conductor durably retires the stale ownership.
Terminal claim/label observation outages retain their transient classification and replay the
same completed invocation; an unchanged authoritative claim ref after a failed push does not
retire ownership.

The fixed `.autospec/executor-result.json` ingestion and bare
`executor-result --repo OWNER/REPO --issue N` deferred receipt remain compatibility inputs, but
they are no longer the default producer or a terminal conductor result. Explicit executor-result
ingestion is described below. Direct `run-foreground` remains one-cycle for bounded use.
Detached `autonomous start` and `restart` launch it as a direct Rust child with inherited
lifecycle ownership; that child repeats cycles, emits each completed cycle to its scoped log,
re-checks stop and budget admission before the next cycle, and exits only for a named terminal
condition or `--max-cycles`. Monitor and supervisor remain separate compatibility observers.

Every start-family launch refreshes the requested checkout before lifecycle acquisition. A stale
or missing runtime is rebuilt into an immutable source-digest generation, the build is rejected
if its source identity moves, and the launcher executes the exact verified generation path.
Read-only and stop commands do not require a rebuild.

Autonomous accountability is mandatory: every live launch creates or adopts exactly one verified managed run epic before conductor spawn.
A normal `start` creates its own epic. `start --epic N`
adopts only an active issue in the requested repository with the managed marker, recovery manifest,
and `epic`, `type:tracker`, `no-auto`, and `autospec:run-accountability` labels. `autospec autonomous
resume --epic N` may reopen a verified closed or parked epic and reconstruct a chained local journal
segment from its acknowledged recovery manifest. It records `resumed_from_epic` before work and
never attaches to an arbitrary issue. There is no bypass flag or environment variable for the epic
or private journal.

Autospec's marker-bounded epic projection preserves human text and contains short What, Why, and
Evidence paragraphs, linked issues and pull requests, a Mermaid dependency/deliverable flowchart,
a Mermaid run-state diagram, current work, blockers, verification, and next steps. Events are
durable locally before projection; later GitHub edit failures remain visible and retryable without
creating a replacement epic. Optional GitHub Project assignment may organize the issue but never
replaces it or blocks launch after the epic itself is verified.

`autospec autonomous status --json` and `autospec autonomous list --json` read accountability
health locally. Their accountability object includes `run_id`, `epic_number`, `epic_url`, `event_count`,
`pending_projection_count`, desired and acknowledged high watermarks, projection state, and any
local error. A missing or ambiguous marker, invalid recovery manifest, lost lifecycle lease, or
local journal failure blocks spawn or the next work mutation rather than silently replacing state.

`autospec autonomous main-health` and `run-foreground` read the strict
repository-local Rust health policy at `<repo-dir>/.autospec/autonomous.yml`.
The optional `--branch` override takes precedence over that file and then the
GitHub default branch; exact ignored names become advisory health evidence only.
See [mainline health admission](runbooks/mainline-health-admission.md) and the
[configuration reference](CONFIG_REFERENCE.md#repository-local-rust-mainline-health)
for the supported schema and fail-closed behavior.

`autospec autonomous blast-radius` reads newline-delimited changed paths from
`--changed-files` and matches them against `.autospec/autospec.yml`
`fenced_surfaces` by default, or a caller-supplied `--fenced-surfaces` registry.
Fenced matches emit `blast:fenced`, `decision:"quarantine"`, and non-zero exit
status so policy config changes such as `.autospec/autospec.yml` cannot be
treated as low-blast-radius.

`autospec autonomous drain --repo OWNER/REPO --repo-dir DIR [--stall-secs N] [--poll-secs N]
[--json]` is the Rust Tier-1 watchdog for the fixed direct `omx exec ... $autospec-run` child.
It forwards child output, resets the stall window when output, progress artifacts, scoped
heartbeats, or the timeout-boundary GitHub snapshot advances, and returns the child’s actual exit
status if it completes. External-only progress emits one
`quiet_stdout_external_progress` warning; a genuinely silent, live child emits a
`terminate_stalled` decision and exits `124`. Each decision is atomically recorded as
`drain-observation.json` in the repository’s autonomous-operator scope, without raw output or
lease tokens. The command uses direct `omx`, `gh`, and process arguments—never a shell or a
legacy drain fallback. Existing shell launcher wiring is intentionally a separate #2076 deletion
child; it must redirect to this command rather than reimplement this watchdog.

`autospec autonomous resilience --help` describes the one supported action and its canonical
write slug, `owner__repo`. `resilience decide` reads state, per-issue failure, and spend records
for `owner/repo` in the strict order `owner__repo`, `owner_repo`, then `owner-repo`. The first
existing record is authoritative: malformed records fail closed instead of falling through, and a
state record for another repository returns the `foreign_state` rejection. Read-only admission
does not migrate a compatibility layout. Every acquisition, adoption, or release targets only
`owner__repo`; neither legacy layout is written. The separate
`.autospec/autonomous-operator/<scope>/` directory is lifecycle-only and never stores resilience
compatibility state.

The command prints exactly one JSON decision. `available` and `reclaim` exit `0`; `held` and
capacity parks exit `20`; malformed, foreign, or failure-cap rejections exit `3`. Claimed leases
are reclaimable at the inclusive 300-second boundary and all leases at the inclusive 10,800-second
abandoned boundary (with missing heartbeats and dead same-host PIDs also reclaimable). Capacity is
also inclusive: a nonzero usage limit is evaluated first, then a nonzero issue limit; `0` disables
the corresponding limit. This adapter reads and evaluates state only: it does not invoke
`scripts/autonomous-resilience.sh`, `sh`, or `bash`.

`start` and `restart` finish read-only repository, stored-stop, and lifetime-budget validation
before atomically taking the local non-blocking Unix lease at
`autonomous/owner__repo/conductor.lease.lock`. Only then may they create operator directories,
persist lifecycle/launch metadata, terminate a unit, or clear a stop flag. A fresh held lease
parks with `conductor_lease_held` (`20`), so a held `restart` cannot kill a process or remove a
stop flag. Capacity/failure policy results retain their park/reject JSON and do not create a
claimed lease. The opaque claimed token and monotonic generation fence delayed children: native
launch passes the token only in `AUTOSPEC_CONDUCTOR_LEASE_TOKEN` through `Command::env`, never in
arguments, launch JSON, or logs. A launch failure terminates already-started children and releases
only the matching lease.

Foreground first honors a persisted stop for executable work. When a launcher supplied a token, it
first adopts that token solely to gain matching release authority, then releases it before returning
the persisted-stop or a pre-admission diagnostic. Otherwise it atomically adopts the environment
token (or, when absent, acquires its own) before lifecycle, health, queue, claim, or foreground-state
work. `conductor_lease_token_mismatch` is a reject (`3`) before any local or GitHub mutation.
Matching ownership is rechecked for final selection and dispatch, preserving their admission gates;
terminal foreground work persists its decision before releasing only its exact matching token.
`autonomous status --json` reports the same scoped
`AUTOSPEC_AUTONOMOUS_SPEND_DIR/<owner__repo>/spend.json` ledger used by admission, not the retired
global spend file. Both `autonomous status --json` and `autonomous list --json` also include a
top-level `toolchain` object with `installed_version`, `remote_version`,
`installed_age_secs`, `last_update_failed`, and `last_update_failure_path`. The version and failure
records are read from `~/.autospec/`; missing data is represented explicitly as JSON `null` or
`false`. Both background `autonomous start` and direct foreground entry warn without blocking when
the failure record exists. I/O and transaction failures are diagnostics (`2`) with no decision
JSON, while malformed/foreign records and token fencing are
JSON rejects (`3`). The local lease coordinates only the shared filesystem; GitHub claim ownership
remains the remote mutation arbiter.

`autospec autonomous lifecycle decide` evaluates the typed repository scope, issue, worker,
claim branch, lease freshness, stop, ownership, retry, health, budget, waterfall-tier, and
idle-rescan policy without any side effects. A claim requires its complete typed identity
(`--claim-repo`, `--claim-issue`, `--claim-worker`, and `--claim-branch`); `--claim-state
terminal` and lease ages above 10,800 seconds have distinct non-executable decisions. `--repo`
is required. It emits exactly one JSON decision: `run` exits `0`, `stop` and `park` exit `20`,
claim or scope rejection (including malformed observed claim state) exits `3`, and malformed
flags exit `2`. `start`, `restart`,
`run-foreground`, and `stop` write the same collision-safe atomic schema-1
`.autospec/autonomous-operator/<scope>/lifecycle.json` decision record. Start and restart
launch conductor, monitor, and supervisor as direct Rust executable-plus-argument-vector
children; they do not accept command-string companion overrides or use `sh -c`. Foreground
reads a stop flag or stored stop record before health or queue work and between launched cycles,
returns the same JSON decision and exit class for health parks, and preflights the observed
GitHub claim before any claim label, heartbeat, or run-state mutation.

`autospec autonomous executor-result` emits one JSON result and has two deliberately distinct
forms. The bare compatibility form is exactly `--repo OWNER/REPO --issue N`: it is the successful
legacy deferred receipt above and exits `0`. An explicit result must include a repository, a
positive issue number, `--worker-id`, `--branch`, and `--outcome`. Its fields are strict: unknown
or repeated flags, or mixed outcome fields, are malformed. `succeeded` requires a positive `--pr`
and forbids `--reason`; `blocked` and `retryable` require a nonempty `--reason` and forbid `--pr`.

An explicit successful result is accepted only when its worker ID and branch match a fresh,
nonterminal claim, and its PR remains open, closes the issue, contains exactly one
`## Closeout report` heading, and has that same branch as its head ref. Rust appends an immutable
receipt and re-reads the active claim before accepting it; it never patches the shared claim for an
explicit outcome. That evidence is not release or merge authority. JSON exit codes are
`0` for accepted success or the legacy deferred receipt, `10` for retryable, `20` for blocked or
evidence-unavailable, `2` for malformed input, and `3` for ownership lost.
`result_recording_failed` is also a blocked (`20`) reason: evidence creation or confirmation
failed, so any receipt is unconfirmed/inert, the shared claim is unchanged, and callers must not
treat it as a recorded executor-blocked outcome.
