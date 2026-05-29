#!/usr/bin/env bash
# scripts/lib/explore-internet-safety.sh — shared safety helpers for the
# /autospec-explore internet researcher (issue #720).
#
# Provides:
#   explore_canonicalize_url          — canonicalize a URL into host + path
#   explore_internet_domain_allowed   — assert host is in the allowlist; on
#                                       miss prints
#                                         code_health:explore_forbidden_domain
#                                       and returns 1.
#   explore_internet_injection_guard  — sanitize fetched content; on hit prints
#                                         code_health:explore_injection_detected
#                                       and returns 1.
#   explore_internet_rate_check       — bump counters in the rate-history file
#                                       and reject when per-round (5) or
#                                       per-session (30) cap exceeded.
#
# Defaults configurable via env:
#   AUTOSPEC_EXPLORE_INTERNET_HISTORY  default: ~/.autospec/explore-internet-history.json
#   AUTOSPEC_EXPLORE_INTERNET_ROUND_ID default: auto (current round)
#   AUTOSPEC_EXPLORE_INTERNET_FETCH_CAP_PER_ROUND   default 5
#   AUTOSPEC_EXPLORE_INTERNET_FETCH_CAP_PER_SESSION default 30
#
# The history file stores only sha256(URL) + timestamp (PR #704 pattern).

if [ -n "${_AUTOSPEC_EXPLORE_INTERNET_SAFETY_LOADED:-}" ]; then
    return 0 2>/dev/null || true
fi
_AUTOSPEC_EXPLORE_INTERNET_SAFETY_LOADED=1

# Default allowlist if operator did not supply --internet-allowlist.
EXPLORE_DEFAULT_ALLOWLIST="github.com,raw.githubusercontent.com,docs.github.com,developer.mozilla.org,nodejs.org,python.org,docs.python.org"

# explore_canonicalize_url <url> → "<host>\t<path>"
explore_canonicalize_url() {
    local url="$1"
    [ -n "$url" ] || return 1
    # Strip scheme.
    local rest="${url#http://}"
    rest="${rest#https://}"
    rest="${rest#//}"
    # Reject empty after strip.
    [ -n "$rest" ] || return 1
    # Split host / path.
    local host="${rest%%/*}"
    local path=""
    case "$rest" in
        */*) path="/${rest#*/}" ;;
        *) path="/" ;;
    esac
    # Lowercase host.
    host="$(printf '%s' "$host" | tr '[:upper:]' '[:lower:]')"
    # Strip user:pass@ and port.
    host="${host##*@}"
    host="${host%%:*}"
    # Disallow empty / suspicious hosts.
    case "$host" in
        ""|*"/"*|*" "*) return 1 ;;
    esac
    printf '%s\t%s' "$host" "$path"
}

# explore_internet_domain_allowed <url> <allowlist-csv>
# Returns 0 if the URL host is in the allowlist, 1 otherwise.
explore_internet_domain_allowed() {
    local url="$1"
    local allowlist="${2:-$EXPLORE_DEFAULT_ALLOWLIST}"
    local parsed host
    parsed="$(explore_canonicalize_url "$url")" || {
        echo "code_health:explore_forbidden_domain url=<unparseable>" >&2
        return 1
    }
    host="${parsed%%	*}"
    # Iterate allowlist entries; allow exact match OR subdomain match.
    local IFS=','
    local entry
    for entry in $allowlist; do
        entry="$(printf '%s' "$entry" | tr -d '[:space:]' | tr '[:upper:]' '[:lower:]')"
        [ -n "$entry" ] || continue
        if [ "$host" = "$entry" ]; then
            return 0
        fi
        case "$host" in
            *.${entry}) return 0 ;;
        esac
    done
    echo "code_health:explore_forbidden_domain host=$host" >&2
    return 1
}

# explore_internet_injection_guard <content>
# Returns 0 if content is clean, 1 if injection detected.
# Categories logged WITHOUT offending content:
#   prose_override — "ignore previous" / "disregard prior" / "you are now" /
#                    "system prompt" / "new instructions" / "</?system>"
#   shell_metachar — line begins with $( , `, &&, ;, |, >
explore_internet_injection_guard() {
    local text="$1"
    if printf '%s' "$text" | grep -qiE '(ignore previous|disregard prior|you are now|system prompt|new instructions|</?system>)'; then
        echo "code_health:explore_injection_detected category=prose_override" >&2
        return 1
    fi
    if printf '%s\n' "$text" | grep -qE '^[[:space:]]*(\$\(|&&|;|\||>)'; then
        echo "code_health:explore_injection_detected category=shell_metachar" >&2
        return 1
    fi
    if printf '%s\n' "$text" | grep -qE '^[[:space:]]*`([^`]|$)'; then
        echo "code_health:explore_injection_detected category=shell_metachar" >&2
        return 1
    fi
    return 0
}

# explore_internet_rate_check <url>
# Mutates the rate-history file. Returns 0 if within cap, 1 if exceeded.
# Stores sha256(url) + timestamp + round_id (no raw URLs).
explore_internet_rate_check() {
    local url="$1"
    local history_file="${AUTOSPEC_EXPLORE_INTERNET_HISTORY:-$HOME/.autospec/explore-internet-history.json}"
    local round_id="${AUTOSPEC_EXPLORE_INTERNET_ROUND_ID:-default}"
    local cap_round="${AUTOSPEC_EXPLORE_INTERNET_FETCH_CAP_PER_ROUND:-5}"
    local cap_session="${AUTOSPEC_EXPLORE_INTERNET_FETCH_CAP_PER_SESSION:-30}"
    mkdir -p "$(dirname "$history_file")" 2>/dev/null || true
    if [ ! -f "$history_file" ]; then
        printf '%s\n' '{"entries":[]}' > "$history_file"
    fi
    if ! command -v python3 >/dev/null 2>&1; then
        # No python3 → silently allow (extreme degraded env).
        return 0
    fi
    python3 - "$history_file" "$url" "$round_id" "$cap_round" "$cap_session" <<'PY'
import json, sys, os, hashlib, time
path, url, round_id, cap_round, cap_session = sys.argv[1:6]
cap_round = int(cap_round); cap_session = int(cap_session)
try:
    with open(path, 'r', encoding='utf-8') as fh:
        data = json.load(fh)
except Exception:
    data = {"entries": []}
entries = data.get("entries", [])
# Filter session window = last 24h.
now = time.time()
session_cutoff = now - 86400
entries = [e for e in entries if isinstance(e, dict) and e.get("ts", 0) > session_cutoff]
round_count = sum(1 for e in entries if e.get("round") == round_id)
session_count = len(entries)
if round_count >= cap_round:
    sys.stderr.write("code_health:explore_rate_limit_exceeded scope=round round=%s count=%d cap=%d\n" % (round_id, round_count, cap_round))
    sys.exit(1)
if session_count >= cap_session:
    sys.stderr.write("code_health:explore_rate_limit_exceeded scope=session count=%d cap=%d\n" % (session_count, cap_session))
    sys.exit(1)
h = hashlib.sha256(url.encode('utf-8')).hexdigest()
entries.append({"hash": h, "ts": now, "round": round_id})
data["entries"] = entries
tmp = path + ".tmp"
with open(tmp, 'w', encoding='utf-8') as fh:
    json.dump(data, fh)
os.replace(tmp, path)
PY
    return $?
}
