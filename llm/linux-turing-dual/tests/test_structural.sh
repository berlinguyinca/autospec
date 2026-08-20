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
  # The gateway's own list is separate from the router's, but the example must
  # still document it -- otherwise a gateway operator gets exit 78 with no
  # template to fill in.
  demanded="${demanded} $(sed -n 's/^QT_GATEWAY_REQUIRED_VARS="\(.*\)"$/\1/p' "${NODE}/scripts/site.sh")"
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

# --- 9b: NO client-reachable location may reach llama.cpp directly --------
# The gateway is what authenticates inference. A location that proxies straight
# to the runtime keeps working perfectly -- and unauthenticated. That is the
# failure this check exists for, and it nearly shipped: the queue-header
# fail-open fallback pointed at llama.cpp, so a gateway crash would have opened
# the endpoint. Comments are stripped first, so documenting the upstream is fine.
if [ -r "$ngx" ]; then
  if sed 's/#.*$//' "$ngx" | grep -qE 'proxy_pass[[:space:]]+https?://qwen_llama'; then
    bad "nginx proxies to llama.cpp directly -- that bypasses authentication"
  else
    ok "nginx: no location reaches llama.cpp directly"
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

# --- every module the shipped scripts import is installed -------------------
# Derived from the scripts' OWN import lines rather than kept as a second list:
# usage.py was left out of the installer once and upstreams.py a second time, and
# both are invisible until a CLEAN install dies at import.
#
# TRANSITIVELY, because a module is not only imported by the entrypoint: tunnel.py
# imports wsframe, and checking gateway.py alone would have missed it.
missing=""
for f in "${NODE}"/scripts/*.py; do
  # Only modules that are actually shipped can drag in a dependency that must be.
  grep -Eq "install .*$(basename "$f")" "${NODE}/scripts/install-node.sh" || continue
  while read -r mod; do
    [ -n "${mod}" ] || continue
    [ -f "${NODE}/scripts/${mod}.py" ] || continue
    # An `install` LINE, not any mention: the comment above that block names the
    # modules it forgot, and matching prose would make this check vacuous. It was,
    # for one commit, until a positive control caught it.
    grep -Eq "install .*${mod}\\.py" "${NODE}/scripts/install-node.sh" \
      || missing="${missing} ${mod}.py (needed by $(basename "$f"))"
  done <<EOF
$(sed -n 's/^import \([a-z_]*\) as .*/\1/p; s/^import \([a-z_]*\)$/\1/p; s/^from \([a-z_]*\) import .*/\1/p' "$f")
EOF
done
if [ -n "${missing}" ]; then
  bad "installer would not ship:${missing}"
else
  ok "installer ships every module the shipped scripts import"
fi

# --- the new-key reveal must survive its own refresh ------------------------
# The bug this guards: doMint() wrote the key into #minted and then called
# loadKeys(), which rewrites the whole keysbox -- #minted included. The reveal
# was destroyed in the same task that created it, so a newly created key was
# never visible for even one frame, with no way to recover it. Reproduced in a
# browser, not inferred.
#
# The invariant is ownership: #minted lives inside loadKeys()'s template, so
# loadKeys() is the only function that may fill it. Anything writing it directly
# is the bug coming back.
page="${NODE}/web/index.html"
if grep -q "\$('minted')\.innerHTML" "$page"; then
  bad "web/index.html writes #minted directly; it must render from state via mintedBox()"
elif ! grep -q "id=\"minted\">' + mintedBox()" "$page"; then
  bad "web/index.html must render the key reveal from state inside the keys panel"
else
  ok "the new-key reveal is rendered by the panel that owns it"
fi

# --- the agent endpoints, and the invariant that keeps them safe ------------
conf="${NODE}/nginx/qwen-turing.conf"
agent_block="$(awk '/location \^~ \/api\/agent\//,/^    }/' "$conf")"
if [ -z "$agent_block" ]; then
  bad "nginx has no /api/agent/ location, so no server could ever attach"
elif ! printf '%s' "$agent_block" | grep -q "proxy_pass http://qwen_gateway"; then
  # The dashboard has no idea what a server credential is; sending agent traffic
  # there would fail in a way that looks like a protocol bug.
  bad "nginx must proxy /api/agent/ to the gateway, not the dashboard"
elif ! printf '%s' "$agent_block" | grep -q 'proxy_set_header Upgrade'; then
  bad "nginx must pass the Upgrade header on /api/agent/, or no WebSocket forms"
else
  ok "nginx routes the agent endpoints to the gateway with an upgrade"
fi

# The idle-pipe keepalive must stay INSIDE nginx's read timeout, or a pipe that
# sits unused dies and the first request after a quiet hour fails on a reset.
# Both numbers are parsed rather than asserted as literals, so editing either one
# is caught instead of the pair drifting apart.
ka="$(sed -n 's/^PIPE_KEEPALIVE_SECONDS *= *\([0-9]*\).*/\1/p' "${NODE}/scripts/gateway.py")"
rt="$(printf '%s' "$agent_block" | sed -n 's/.*proxy_read_timeout *\([0-9]*\)s.*/\1/p' | head -1)"
if [ -z "$ka" ] || [ -z "$rt" ]; then
  bad "could not read the pipe keepalive ($ka) or nginx read timeout ($rt)"
elif [ "$ka" -ge "$rt" ]; then
  bad "pipe keepalive ${ka}s must be well inside nginx's ${rt}s read timeout"
else
  ok "idle pipes are pinged (${ka}s) inside nginx's read timeout (${rt}s)"
fi

# THE NODE NEVER TELLS AN AGENT WHERE TO CONNECT. The agent's target comes from
# its own config file, so a compromised node cannot turn every attached agent
# into a port scanner inside its owner's network. The mechanism is that the node
# sends no text frames at all on the control connection -- only ping, pong and
# close -- so this asserts exactly that.
# Matching the WRITE, not any mention: the control loop legitimately READS text
# frames (that is how a server describes itself), and a grep for OP_TEXT alone
# flagged that -- a check that fails on correct code gets deleted, not fixed.
if grep -Eq "encode\(_ws\.OP_TEXT|encode\(OP_TEXT" "${NODE}/scripts/gateway.py"; then
  bad "the gateway sends a text frame to an agent; it must send it no instructions"
else
  ok "the node sends an agent no instructions, so it cannot name a destination"
fi

# --- the agent builds everywhere, with nothing beside it -------------------
# Skipped rather than failed when Go is absent: this suite must stay runnable on
# a node that only serves inference. CI has Go and therefore runs it.
agent="$(dirname "${NODE}")/agent"
if [ ! -f "${agent}/go.mod" ]; then
  bad "the agent source is missing"
elif ! command -v go >/dev/null 2>&1; then
  ok "agent build skipped (no Go toolchain here)"
elif ! (cd "$agent" && go vet ./... >/dev/null 2>&1); then
  bad "go vet fails in the agent"
elif grep -Eq "^[[:space:]]*require" "${agent}/go.mod"; then
  # Read from go.mod rather than from `go list -m all`: an UNRESOLVED require
  # makes go list fail, so its output is empty -- and an empty list read as "no
  # dependencies". The manifest cannot lie that way, and this works without a
  # toolchain at all. One dependency is all it takes to stop being a single file
  # you can copy onto a machine.
  bad "the agent has grown a dependency; it must be standard library only"
else
  ok "the agent vets clean with no dependencies"
fi

echo
if [ "$fails" -eq 0 ]; then

  echo "OK -- all structural checks passed"
  exit 0
fi
echo "${fails} structural check(s) failed" >&2
exit 1
