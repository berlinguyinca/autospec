# Release Checklist

Use this before tagging or announcing an AutoSpec release.

- [ ] `bash scripts/validate.sh --fast` passes.
- [ ] `bash scripts/validate-launch-readiness.sh` passes.
- [ ] `bash scripts/validate-public-launch-readiness.sh` passes.
- [ ] README quickstart still works from a fresh clone.
- [ ] `examples/hello-autospec/` demo is readable without private context.
- [ ] `CHANGELOG.md` has a current release entry.
- [ ] `ROADMAP.md` names the next likely release focus.
- [ ] `SECURITY.md` and `SAFETY.md` are present.
- [ ] Launch posts under `marketing/` match the current README.

