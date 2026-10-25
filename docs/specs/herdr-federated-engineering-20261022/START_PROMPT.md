Implement the attached Herdr Federated Engineering specification across existing repositories. The archive is on host `beast` in its Downloads folder. You may be running on ANY machine. First retrieve it to the machine you actually run on, without assuming beast's home directory or a shared filesystem:

```bash
set -euo pipefail
SPEC=herdr-federated-engineering-20261022
mkdir -p "$HOME/.cache/herdr-specs"
if [ -f "$HOME/Downloads/$SPEC.zip" ]; then
  cp "$HOME/Downloads/$SPEC.zip" "$HOME/.cache/herdr-specs/$SPEC.zip"
else
  REMOTE_PATH=$(ssh beast 'for p in "$HOME/Downloads/herdr-federated-engineering-20261022.zip" "$HOME/downloads/herdr-federated-engineering-20261022.zip" /Downloads/herdr-federated-engineering-20261022.zip; do if test -f "$p"; then printf "%s\n" "$p"; exit 0; fi; done; exit 2')
  scp "beast:$REMOTE_PATH" "$HOME/.cache/herdr-specs/$SPEC.zip"
fi
unzip -t "$HOME/.cache/herdr-specs/$SPEC.zip"
```

If `beast` SSH is unreachable, report the exact blocker; do not invent success. If archive is absent on beast, request that it be copied to beast's Downloads directory. Discover actual local checkouts (`git remote -v`) or clone the canonical repositories into an isolated workspace. For each target repo create a UNIQUE `docs/specs/herdr-federated-engineering-20261022/` path and extract a copy there; never unzip into repo root or overwrite other specs. Work in feature branches and independent worktrees. Start with T00 source audit, exact file/function/test evidence, current git SHA, and REUSE/EXTEND/CONSOLIDATE/NEW matrix. Then execute TASKS.json in dependency order, parallelizing independent tasks when safe. Maintain a progress report with completed/remaining tasks and evidence-backed ETA range. Follow each repo's AGENTS.md. Reuse existing Pi ledger/mission adapter, Herdr HCN/event sink/CI/reviews and AutoSpec review/quorum. Preserve standalone Pi with AutoSpec off. Do not use InferWeave/inferweave-dashboard or InferWeave/inferweave-workbench. No duplicate orchestrator, event store, collaboration service or dashboard. Run acceptance scenarios, independent review and CI. Open PRs; do not push directly to main. Continue until verified or clearly blocked.
