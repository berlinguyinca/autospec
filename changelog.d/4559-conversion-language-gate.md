# Conversion pass language gate (issue #4559)

The Rust gate (`fmt`/`build`/`clippy`/`test`) cannot fail a patch it cannot
see, so a gate that cannot fail a patch must not report passing it. Before
any branch is created, the conversion pass classifies each patch's changed
files into `rust-go` (the gate's domain), `shell`, `mixed`, or `neither`
(`autospec_core::patch_language`), and the ungatetable classes are held
`unevaluated`:

- **shell** — the standing ruling (#4447, shell→Rust) is cited by the hold;
- **mixed** — the shell files are named: a shell file in a Rust prompt is a
  prompt signal, not a conversion candidate;
- **neither** — docs/fixtures/config only: an explicit decision, not a
  default fall-through (the verdict is `unevaluated`, never `pass`).

The hold is a separate axis from attempt-state disqualification (branch /
PR / held): `select_fresh` now reports `fresh` / `disqualified` /
`language_held` and the selection line counts holds per language. On
`--apply` a language-held patch is archived to
`out/issue-N/language-held/changes-<ts>.patch` — terminal, its queue entry
freed, the pass never re-offers it — while a stale held record still goes to
`superseded/` and returns to the pool when re-dispatched.

The pass's outcome counters also stop lying: a pass that examined and held
patches reports them as `skipped` instead of the idle
`converted=0 held=0 skipped=0` line.
