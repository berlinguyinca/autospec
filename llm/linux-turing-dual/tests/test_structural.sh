#!/usr/bin/env bash
# No-accelerator structural checks for the dual-Turing node. Runs in CI.
#
# This is the suite that would have caught a hostname reaching a public
# repository, so it must be cheap enough to run on every push and must not need
# a GPU, a server, or the weights.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NODE="$(dirname "$HERE")"
root="$(git -C "$NODE" rev-parse --show-toplevel 2>/dev/null || echo "")"

fails=0
ok()  { echo "ok   -- $*"; }
bad() { echo "FAIL -- $*" >&2; fails=$(( fails + 1 )); }

# --- 1: every shipped script must exist and parse -------------------------
for s in site.sh vram-guard.sh; do
  f="${NODE}/scripts/${s}"
  if [ ! -r "$f" ]; then
    bad "missing scripts/${s}"
  elif bash -n "$f" 2>/dev/null; then
    ok "scripts/${s} parses"
  else
    bad "scripts/${s} has a syntax error"
  fi
done

# --- 2: the example must cover every value require_site() demands ---------
# Otherwise a fresh operator gets exit 78 and no way to know what to write.
ex="${NODE}/config/site.conf.example"
if [ ! -r "$ex" ]; then
  bad "missing config/site.conf.example"
else
  demanded="$(sed -n 's/^QT_REQUIRED_VARS="\(.*\)"$/\1/p' "${NODE}/scripts/site.sh")"
  if [ -z "$demanded" ]; then
    bad "could not read QT_REQUIRED_VARS from scripts/site.sh"
  else
    missing=""
    for v in $demanded; do
      grep -q "$v" "$ex" || missing="${missing} ${v}"
    done
    [ -z "$missing" ] \
      && ok "site.conf.example covers every demanded value" \
      || bad "site.conf.example does not set:${missing}"
  fi
fi

# --- 3: LEAK GUARD --------------------------------------------------------
# This repository is public. Patterns come from the environment so that this
# file does not itself contain the identifiers it forbids -- a guard that
# names the secret is not a guard.
#
#   QT_LEAK_PATTERNS='10\.0\.0\.5|myhost' tests/test_structural.sh
leak_pat="${QT_LEAK_PATTERNS:-}"
if [ -z "$leak_pat" ]; then
  ok "leak guard skipped (QT_LEAK_PATTERNS unset)"
elif [ -z "$root" ]; then
  bad "leak guard cannot run: not a git checkout"
else
  hits="$(git -C "$root" grep -niIE "$leak_pat" -- llm docs 2>/dev/null \
          | grep -v 'leak-guard-allow' || true)"
  if [ -n "$hits" ]; then
    bad "a real site identifier is committed:"
    echo "$hits" | sed 's/^/        /' | head -5 >&2
  else
    ok "no real site identifier is committed"
  fi
fi

# --- 4: committed config must keep placeholders, not real values ----------
literal=0
for f in "${NODE}"/config/*.example; do
  [ -r "$f" ] || continue
  # A dotted quad or a bare hostname as a DEFAULT means someone pasted a real value.
  if grep -qE ':=[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}' "$f"; then
    bad "$(basename "$f") hardcodes a literal IP address"
    literal=1
  fi
done
[ "$literal" -eq 0 ] && ok "committed config carries no literal address"

# --- 5: the CUDA sandboxing ceiling must not be tightened back -----------
# Each of these looks like hardening and produces a unit that restart-loops.
u="${NODE}/systemd/qwen-turing@.service"
if [ ! -r "$u" ]; then
  ok "unit not present yet (checked once systemd/ is populated)"
else
  for forbidden in 'ProtectSystem=strict' 'PrivateDevices=true' \
                   'DevicePolicy=closed' 'MemoryDenyWriteExecute=yes'; do
    grep -qE "^[[:space:]]*${forbidden}" "$u" \
      && bad "unit sets ${forbidden}, which breaks CUDA on this host"
  done
  grep -qE '^[[:space:]]*ProtectSystem=full' "$u" \
    && ok "unit uses ProtectSystem=full" \
    || bad "unit must set ProtectSystem=full"
  grep -qE '^[[:space:]]*LoadCredential=' "$u" \
    && ok "unit takes the API key as a credential" \
    || bad "unit must load the API key via LoadCredential, not Environment="
  grep -qE '^[[:space:]]*Environment=.*(API_KEY|apikey)' "$u" \
    && bad "unit exposes the API key via Environment= (readable by systemctl show)"
fi

# --- 6: cross-slot KV reuse must stay off -------------------------------
p="${NODE}/config/profiles.d/router.conf"
if [ -r "$p" ]; then
  if grep -qE '^[[:space:]]*QT_SLOT_PROMPT_SIMILARITY="?0\.0"?' "$p"; then
    ok "cross-slot KV reuse is off"
  else
    bad "QT_SLOT_PROMPT_SIMILARITY must be 0.0 -- the non-zero path crashes the model child"
  fi
fi

echo
if [ "$fails" -eq 0 ]; then
  echo "OK -- all structural checks passed"
  exit 0
fi
echo "${fails} structural check(s) failed" >&2
exit 1
