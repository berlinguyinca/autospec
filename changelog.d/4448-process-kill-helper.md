### Added

- `autospec process-kill <pattern> [--signal <NAME>]` — the documented
  pattern-based process termination (issue #4448). `pkill -f <pattern>`
  matches the invoking shell's own command line and dies with exit 144,
  taking the work queued after the kill with it; this helper brackets the
  pattern's first character automatically, excludes its own session
  (itself and every ancestor), and kills the remaining matches by pid,
  reporting a zero-match kill as a false negative instead of a silent
  success. New module `autospec_core::process_termination`
  (`bracket_pattern`, `kill_matching`, `KillReport`); `AGENTS.d/4448`
  names exit 144 as the signature and states the self-match mechanism, and
  the process-exclusivity runbook points pattern-based kills at the helper.
