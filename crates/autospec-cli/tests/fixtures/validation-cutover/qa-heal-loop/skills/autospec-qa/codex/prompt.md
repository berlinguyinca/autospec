## Self-healing loop
## --no-heal opt-out
qa-heal-loop.sh qa-finding-to-issue.sh
oscillation_detected AUTOSPEC_HEAL_MAX_ROUNDS
default-on --single-pass qa-no-heal.flag lib/autospec-loop.sh qa-heal-summary.md evidence_based_stop
