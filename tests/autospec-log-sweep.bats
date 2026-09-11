#!/usr/bin/env bats
# tests/autospec-log-sweep.bats — issue #4246: a log's last line is only
# "now" if its mtime says so. `tail`/`grep` answer "what was written last",
# which is a different question from "what is happening", and the two
# diverge exactly when a component has stopped — the case under
# investigation. A supervisor log appended to only on failure shows its last
# failure, arbitrarily far in the past, as though it were the present (a
# syntax error 3 days old in such a log was read as a current outage while
# the sweep that found it had been silent for those same 3 days). So every
# error sweep over a set of logs prints each file's mtime alongside its
# matches — clean files included, since a negative result needs a freshness
# too — and says plainly when a match comes from a file older than the
# investigation window.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
LIB="$REPO_ROOT/scripts/lib/autospec-log-status.sh"

# _backdate FILE SECONDS — set FILE's mtime SECONDS before now (GNU touch
# first, BSD -t fallback), so a test controls the clock without sleeping.
_backdate() {
  local f="$1" s="$2"
  if touch -d "@$(( $(date -u +%s) - s ))" "$f" 2>/dev/null; then
    return 0
  fi
  touch -t "$(date -u -v-"${s}"S +%Y%m%d%H%M.%S)" "$f"
}

# _stamp FILE YYYYMMDDhhmm — an absolute mtime far in the past.
_stamp() { touch -t "$2" "$1"; }

setup() {
  DIR="$(mktemp -d)"
}

teardown() {
  rm -rf "$DIR"
}

@test "sweep: a stale log's error match is labelled history, not current state" {
  local log
  log="$DIR/cron-regsweep.log"
  printf 'starting sweep\n' > "$log"
  printf 'line 94: unexpected EOF while looking for matching %s\n' "'" >> "$log"
  # 3 days old: outside any sane investigation window.
  _stamp "$log" 202601010000

  run bash -c ". '$LIB'; autospec_log_sweep EOF '$log'"
  [ "$status" -eq 0 ]
  # The mtime is printed with the match, on the same stdout as the match.
  printf '%s\n' "$output" | grep -Eq "^$log mtime=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z age=[0-9]+ freshness=stale matches=1$"
  # And the plain-language flag names the window and the history/state split.
  printf '%s\n' "$output" | grep -q "STALE:.*outside the 3600s investigation window"
  printf '%s\n' "$output" | grep -q "history, not current state"
  printf '%s\n' "$output" | grep -q "^sweep: 1 file(s), 1 match(es): 0 fresh, 1 stale, 0 unreadable; verdict: history-only$"
}

@test "sweep: a match inside the window is live and exits non-zero" {
  local log
  log="$DIR/live.log"
  printf 'sweep done: pool=9\n' > "$log"
  printf 'ERROR: gateway refused\n' >> "$log"

  run bash -c ". '$LIB'; autospec_log_sweep ERROR '$log'"
  [ "$status" -eq 1 ]
  printf '%s\n' "$output" | grep -q "^$log mtime=.* freshness=fresh matches=1$"
  ! printf '%s\n' "$output" | grep -q "STALE"
  printf '%s\n' "$output" | grep -q "^sweep: 1 file(s), 1 match(es): 1 fresh, 0 stale, 0 unreadable; verdict: live$"
}

@test "sweep: the case that misread an outage — stale error and fresh health together" {
  # The stale supervisor log holds the syntax error; the log the sweep
  # actually writes holds the recent healthy runs. The stale file's error
  # must not be able to masquerade as the current state.
  local stale fresh
  stale="$DIR/cron-regsweep.log"
  fresh="$DIR/regsweep.log"
  printf 'line 94: unexpected EOF while looking for matching %s\n' "'" > "$stale"
  _stamp "$stale" 202601010000
  printf 'sweep done: pool=10 gateway=10\n' > "$fresh"

  # The error sweep over the whole set: the only error evidence is the
  # 3-day-old file, so the verdict is history-only, not an outage.
  run bash -c ". '$LIB'; autospec_log_sweep -w 3600 EOF '$stale' '$fresh'"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "^$stale mtime=.* freshness=stale matches=1$"
  printf '%s\n' "$output" | grep -q "^$fresh mtime=.* freshness=fresh matches=0$"
  printf '%s\n' "$output" | grep -q "^sweep: 2 file(s), 1 match(es): 0 fresh, 1 stale, 0 unreadable; verdict: history-only$"

  # The same set swept for the healthy marker: the live file answers, and
  # the stale error does not get to speak for the present.
  run bash -c ". '$LIB'; autospec_log_sweep -w 3600 'pool=10' '$stale' '$fresh'"
  [ "$status" -eq 1 ]
  printf '%s\n' "$output" | grep -q "^sweep: 2 file(s), 1 match(es): 1 fresh, 0 stale, 0 unreadable; verdict: live$"
  ! printf '%s\n' "$output" | grep -q "STALE"
}

@test "sweep: every match stale means history-only, which is not an outage" {
  local a b
  a="$DIR/a.log"
  b="$DIR/b.log"
  printf 'FATAL: boom\n' > "$a"
  printf 'nothing interesting\nFATAL: older boom\n' > "$b"
  _stamp "$a" 202601010000
  _stamp "$b" 202601020000

  run bash -c ". '$LIB'; autospec_log_sweep FATAL '$a' '$b'"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "^sweep: 2 file(s), 2 match(es): 0 fresh, 2 stale, 0 unreadable; verdict: history-only$"
  [ "$(printf '%s\n' "$output" | grep -c 'STALE:')" -eq 2 ]
}

@test "sweep: a file with no matches still prints its mtime" {
  # A negative result needs a freshness too: "no errors in this log" and
  # "no errors in this log for 3 days" are different findings.
  local log
  log="$DIR/quiet.log"
  printf 'all good\n' > "$log"
  _stamp "$log" 202601010000

  run bash -c ". '$LIB'; autospec_log_sweep ERROR '$log'"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -Eq "^$log mtime=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z age=[0-9]+ freshness=stale matches=0$"
  printf '%s\n' "$output" | grep -q "^sweep: 1 file(s), 0 match(es): 0 fresh, 0 stale, 0 unreadable; verdict: clean$"
}

@test "sweep: an unreadable file is a gap, never a clean result" {
  local log missing
  log="$DIR/ok.log"
  missing="$DIR/absent.log"
  printf 'fine\n' > "$log"

  run bash -c ". '$LIB'; autospec_log_sweep ERROR '$log' '$missing'"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "^$missing freshness=unreadable matches=unknown$"
  ! printf '%s\n' "$output" | grep -q "verdict: clean"
  printf '%s\n' "$output" | grep -q "^sweep: 2 file(s), 0 match(es): 0 fresh, 0 stale, 1 unreadable; verdict: incomplete$"
}

@test "sweep: the window is a parameter, not a constant" {
  local log
  log="$DIR/two-minutes.log"
  printf 'ERROR: recent\n' > "$log"
  _backdate "$log" 120

  # 120s old: stale for a 60s window, fresh for an hour-long one.
  run bash -c ". '$LIB'; autospec_log_sweep -w 60 ERROR '$log'"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "freshness=stale matches=1"
  printf '%s\n' "$output" | grep -q "outside the 60s investigation window"

  run bash -c ". '$LIB'; autospec_log_sweep -w 3600 ERROR '$log'"
  [ "$status" -eq 1 ]
  printf '%s\n' "$output" | grep -q "freshness=fresh matches=1"
}

@test "sweep: age is rendered in the unit a reader compares against" {
  local log
  log="$DIR/three-days.log"
  printf 'ERROR: ancient\n' > "$log"
  _backdate "$log" 259200

  run bash -c ". '$LIB'; autospec_log_sweep ERROR '$log'"
  [ "$status" -eq 0 ]
  # ~3 days, rendered as days rather than a raw second count.
  printf '%s\n' "$output" | grep -q "STALE: last written 3d ago"
}

@test "sweep: a future mtime is fresh, never a negative age" {
  local log
  log="$DIR/skew.log"
  printf 'ERROR: clock skew\n' > "$log"
  _stamp "$log" 203001010000

  run bash -c ". '$LIB'; autospec_log_sweep ERROR '$log'"
  [ "$status" -eq 1 ]
  printf '%s\n' "$output" | grep -q "age=0 freshness=fresh matches=1"
}

@test "sweep: the pattern is a fixed string, not a regex" {
  # The #4246 error text itself contains regex-hostile characters; a sweep
  # that greps it as a regex would silently match nothing.
  local log
  log="$DIR/metachars.log"
  printf 'error [EOF] (closed)\n' > "$log"

  run bash -c ". '$LIB'; autospec_log_sweep '[EOF] (closed)' '$log'"
  [ "$status" -eq 1 ]
  printf '%s\n' "$output" | grep -q "matches=1"
}

@test "sweep: the printed mtime is the file's real mtime, not a formatted now" {
  local log printed
  log="$DIR/real-mtime.log"
  printf 'ERROR: matched\n' > "$log"
  _stamp "$log" 202601010000

  run bash -c ". '$LIB'; autospec_log_sweep ERROR '$log'"
  [ "$status" -eq 0 ]
  printed="$(printf '%s\n' "$output" | awk -v f="$log" '$1 == f { sub(/^mtime=/, "", $2); print $2; exit }')"
  [ -n "$printed" ]
  if ! date -u -d "$printed" +%s >/dev/null 2>&1; then
    skip "date cannot parse RFC3339Z timestamps"
  fi
  [ "$(date -u -d "$printed" +%s)" -eq "$(stat -c %Y "$log" 2>/dev/null || stat -f %m "$log")" ]
}

@test "sweep: usage errors exit 2 and match nothing" {
  run bash -c ". '$LIB'; autospec_log_sweep"
  [ "$status" -eq 2 ]
  run bash -c ". '$LIB'; autospec_log_sweep 'pattern'"
  [ "$status" -eq 2 ]
  run bash -c ". '$LIB'; autospec_log_sweep -w 3600 'pattern'"
  [ "$status" -eq 2 ]
  run bash -c ". '$LIB'; autospec_log_sweep -w"
  [ "$status" -eq 2 ]
  run bash -c ". '$LIB'; autospec_log_sweep -w abc ERROR '$DIR/x.log'"
  [ "$status" -eq 2 ]
}
