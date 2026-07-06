# Release Checklist

Use this before tagging or announcing an AutoSpec release.

- [ ] `bash scripts/validate.sh` passes.
- [ ] `bash scripts/validate.sh --fast` passes for quick local smoke.
- [ ] `bash scripts/validate-launch-readiness.sh` passes.
- [ ] `bash scripts/validate-public-launch-readiness.sh` passes.
- [ ] `cargo run --bin autospec -- --help` lists only commands documented in `docs/cli-reference.md`.
- [ ] README quickstart still works from a fresh clone.
- [ ] `examples/hello-autospec/` demo is readable without private context.
- [ ] `cargo run --bin autospec -- showcase --demo examples/hello-autospec --json` emits local-only JSON.
- [ ] `CHANGELOG.md` has a current release entry.
- [ ] `ROADMAP.md` names the next likely release focus.
- [ ] `SECURITY.md` and `SAFETY.md` are present.
- [ ] Launch posts under `marketing/` match the current README.
