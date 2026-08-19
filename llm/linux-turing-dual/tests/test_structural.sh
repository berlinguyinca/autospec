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

# --- 7: no literal IPv4 anywhere in the node tree ------------------------
# A secret-free companion to check 3. The pattern-based guard needs the real
# identifiers, which must never appear in a committed workflow -- putting them
# there would publish precisely what the guard exists to keep out. This check
# needs no secret: any literal dotted quad under the node directory is wrong,
# because every address this node uses comes from site.conf at runtime.
#
# The filter is Python, not awk: Ubuntu's default awk is mawk, which does not
# support {1,3} interval expressions, so an awk version silently matched nothing
# useful while appearing to work.
#
# Octets are RANGE-CHECKED, because llama.cpp's own log timestamps look like
# dotted quads. A real IPv4 octet is 0-255, which excludes them precisely instead
# of needing a carve-out per fixture. Documentation ranges (RFC 5737), 0.0.0.0 and
# loopback are allowed so examples can show a shape.
addrs="$(
  grep -rIn --include='*.sh' --include='*.conf' --include='*.ini' \
       --include='*.py' --include='*.example' --include='*.service' \
       --include='*.html' --include='*.yaml' \
       -e . "$NODE" 2>/dev/null \
  | grep -v 'leak-guard-allow' \
  | python3 -c '
# Two guards, because either alone lets something through: the octet range kills
# log timestamps like 3.00.332.657, and the leading delimiter kills version
# strings like monero-gui-v0.11.1.0 whose octets are all <=255. leak-guard-allow
import re, sys
QUAD = re.compile(r"(?<![0-9.A-Za-z_-])(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(?![0-9.])")
ALLOW = (("192","0","2"), ("198","51","100"), ("203","0","113"))
for line in sys.stdin:
    for m in QUAD.finditer(line):
        o = m.groups()
        if any(int(x) > 255 for x in o):
            continue                      # a log timestamp, not an address
        if o[:3] in ALLOW or o == ("0","0","0","0") or o[0] == "127":
            continue
        sys.stdout.write(line)
        break
'
)"
# No `|| true` above: this script runs without `set -e`, and the check keys on the
# CONTENT of $addrs, not on the pipeline's status. grep exiting 1 because it found
# nothing is the SUCCESS case here, so masking the status would only hide a real
# failure of python or grep itself.
if [ -n "$addrs" ]; then
  bad "a literal IPv4 address is committed under the node directory:"
  echo "$addrs" | sed 's/^/        /' | head -5 >&2
else
  ok "no literal IPv4 address committed under the node directory"
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

# --- 8: the proxy must not be able to sever long requests -----------------
ngx="${NODE}/nginx/qwen-turing.conf"
if [ -r "$ngx" ]; then
  grep -qE '^[[:space:]]*proxy_buffering[[:space:]]+off' "$ngx" \
    && ok "nginx: response buffering off" \
    || bad "nginx must set proxy_buffering off -- streaming completions break otherwise"
  grep -qE '^[[:space:]]*proxy_request_buffering[[:space:]]+off' "$ngx" \
    && ok "nginx: request buffering off" \
    || bad "nginx must set proxy_request_buffering off -- a 230 KB prompt would buffer whole"
  t="$(sed -n 's/^[[:space:]]*proxy_read_timeout[[:space:]]*\([0-9]\{1,\}\)s;.*/\1/p' "$ngx" | sort -rn | head -1)"
  if [ -n "$t" ] && [ "$t" -ge 600 ]; then
    ok "nginx: proxy_read_timeout ${t}s covers a 100k prefill"
  else
    bad "nginx proxy_read_timeout must be >=600s (100k prefill is ~210s plus generation)"
  fi
  grep -qE '^[[:space:]]*location = /models \{ return 403' "$ngx" \
    && ok "nginx: unsanitised /models denied" \
    || bad "nginx must deny /models -- the unsanitised twin of /v1/models"
  grep -q '@inference_no_headers' "$ngx" \
    && ok "nginx: inference has a no-headers fallback" \
    || bad "nginx must fall back when auth_request fails, or a dead dashboard kills inference"
fi

# --- 9: TLS must stay scaffolded, never half-enabled ----------------------
if [ -r "$ngx" ]; then
  if grep -qE '^[[:space:]]*(return[[:space:]]+301[[:space:]]+https|add_header[[:space:]]+Strict-Transport-Security)' "$ngx"; then
    bad "nginx enables a redirect or HSTS before a certificate exists"
  else
    ok "nginx: no HTTPS redirect or HSTS until the cert lands"
  fi
fi

# --- 10: every page under web/ must be installed by a glob, not by name ----
# status.html was added after index.html and the installer named index.html
# explicitly, so /status returned 500 on a fresh install. A glob ships whatever
# exists; a hardcoded name ships whatever someone remembered.
inst="${NODE}/scripts/install-node.sh"
if [ -r "$inst" ]; then
  if grep -qE 'web/\*\.html' "$inst"; then
    ok "installer ships web pages by glob"
  else
    bad "install-node.sh must install web/*.html by glob, or a new page is silently omitted"
  fi
fi

echo
if [ "$fails" -eq 0 ]; then
  echo "OK -- all structural checks passed"
  exit 0
fi
echo "${fails} structural check(s) failed" >&2
exit 1
