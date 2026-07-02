# Golden Path: Autotrade

1. Run `bash scripts/autospec-v61-status.sh` from Autospec.
2. Use disposable Autotrade clones for any write proof.
3. Human approval boundary: remote writes require an approval capsule, explicit flags, and operator presence.
4. Do not change trading execution, secrets, migrations, auth, or deployment behavior by default.
5. Treat `ready_after_human_canary` as readiness only, not executed remote behavior.
