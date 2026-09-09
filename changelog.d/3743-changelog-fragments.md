### Added

- Changelog is now generated from per-issue fragments under `changelog.d/` (#3743). Agents
  write one new fragment named after their issue instead of hand-editing `CHANGELOG.md`, so
  concurrent agents no longer conflict on the changelog. `scripts/build-changelog.sh` folds
  the fragments into `CHANGELOG.md` as part of the release step (see `changelog.d/README.md`).
