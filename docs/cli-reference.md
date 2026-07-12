# CLI Reference

The Rust CLI is additive. Existing `/autospec-*` skills and shell scripts remain the operational surface while V62+ commands mature.

| Command | JSON | Status |
| --- | --- | --- |
| `autospec init` | no | documented stub, exits non-zero |
| `autospec doctor --json` | yes | implemented |
| `autospec doctor --readiness --json` | yes | implemented target-repo readiness report |
| `autospec status --json` | yes | local status stub |
| `autospec plan --json` | yes | package inspection stub |
| `autospec validate --json` | yes | validation stub pointing to shell gates |
| `autospec runtime classify <path> --json` | yes | implemented R0-R4 ownership classification for one repository path |
| `autospec runtime audit --json` | yes | implemented read-only R0-R4 inventory; it neither migrates nor executes candidates |
| `autospec run` | no | documented stub, exits non-zero |
| `autospec resume` | no | documented stub, exits non-zero |
| `autospec report --json` | yes | report stub |
| `autospec showcase --json` | yes | demo stub |
| `autospec benchmark` | no | documented stub, exits non-zero |
| `autospec growth-report --json` | yes | local-only metrics stub |
