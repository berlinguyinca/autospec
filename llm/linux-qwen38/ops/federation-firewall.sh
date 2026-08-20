#!/usr/bin/env bash
# Restrict this node's inference port to the hosts allowed to federate with it.
#
# WHY THIS EXISTS
#   This node serves inference WITHOUT an API key. That is tolerable only while
#   nothing else can reach the port. Once another node proxies to it -- a
#   federation upstream -- an unauthenticated port reachable from anywhere means
#   any client can bypass that node's authentication entirely, and its usage is
#   attributed to nobody. Restricting the port is what makes the arrangement
#   safe; the proxying node cannot do it for us.
#
# SCOPE, AND THE DEPENDENCY THAT WILL SILENTLY DEFEAT THIS
#   Rules land in the INPUT chain, which is correct while the server runs as a
#   systemd unit on the host. If the server is ever moved into a container,
#   traffic arrives through DOCKER-USER/FORWARD instead, INPUT never sees it,
#   and the port reopens without a single rule changing. Re-point this script
#   at DOCKER-USER if that day comes.
#
#   ufw is deliberately NOT used. This host runs many Docker bridge networks,
#   and enabling ufw sets the FORWARD policy to DROP, which breaks container
#   networking. A targeted chain changes exactly one port's reachability.
#
# USAGE
#   QWEN38_FED_ALLOW="<peer-addr> [<peer-addr>...]" sudo -E ./federation-firewall.sh
#   QWEN38_FED_ALLOW="..." sudo -E ./federation-firewall.sh --persist   # + boot unit
#   sudo ./federation-firewall.sh --status
#   sudo ./federation-firewall.sh --remove
#
#   Addresses are this site's business and are never committed here. Loopback is
#   always allowed so local probes keep working.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
[ -r "${HERE}/../config/common.conf" ] && . "${HERE}/../config/common.conf"

# Every OpenAI-compatible port on this host that federation might expose. This
# host runs two runtimes on different ports by design (see config/common.conf),
# and guarding only the one you happen to be thinking about is how the other
# gets exposed. Override to narrow.
PORTS="${QWEN38_FED_PORTS:-${QWEN38_PORT:-8000} 8080}"
CHAIN="QWEN38-FED"
UNIT="/etc/systemd/system/qwen38-federation-firewall.service"
MODE="${1:-apply}"

need_root() {
  if [ "$(id -u)" -ne 0 ]; then
    echo "must run as root (rules are being changed)" >&2
    exit 77
  fi
}

show() {
  echo "guarded ports: ${PORTS}"
  echo "chain ${CHAIN}:"
  iptables -L "$CHAIN" -n -v 2>/dev/null || echo "  chain absent -- ports are UNRESTRICTED"
  echo "hooks in INPUT:"
  iptables -S INPUT 2>/dev/null | grep -- "$CHAIN" || echo "  not hooked -- ports are UNRESTRICTED"
}

remove() {
  need_root
  local p
  for p in $PORTS; do
    while iptables -C INPUT -p tcp --dport "$p" -j "$CHAIN" 2>/dev/null; do
      iptables -D INPUT -p tcp --dport "$p" -j "$CHAIN"
    done
  done
  iptables -F "$CHAIN" 2>/dev/null || true
  iptables -X "$CHAIN" 2>/dev/null || true
  echo "removed; ports ${PORTS} are now unrestricted"
}

apply() {
  need_root
  if [ -z "${QWEN38_FED_ALLOW:-}" ]; then
    echo "QWEN38_FED_ALLOW is empty -- refusing to apply." >&2
    echo "Applying with no peers would leave a DROP-only chain and cut the" >&2
    echo "upstream off, which is a worse failure than the one being fixed." >&2
    exit 78
  fi

  # VALIDATE EVERY PEER BEFORE TOUCHING A SINGLE RULE.
  #
  # This ordering is the whole point. An earlier version flushed the chain and
  # then validated peers as it added them -- so a bad peer aborted PART WAY
  # THROUGH, leaving a chain holding only the loopback rule: no peer, no DROP,
  # nothing matching, so every packet fell through the chain and was accepted by
  # the INPUT policy. A validation failure silently OPENED the port it exists to
  # close. Caught by the negative test, which is why there is one.
  local peer validated=""
  for peer in $QWEN38_FED_ALLOW; do
    case "$peer" in
      *'<'*'>'*)
        echo "refusing placeholder peer '${peer}' -- rules unchanged" >&2
        exit 78 ;;
      *[!0-9./]*)
        echo "peer '${peer}' is not an IPv4 address or CIDR -- rules unchanged" >&2
        exit 78 ;;
    esac
    validated="${validated} ${peer}"
  done
  if [ -z "$validated" ]; then
    echo "no valid peers survived validation -- rules unchanged" >&2
    exit 78
  fi

  # Rebuilt from scratch every run, so re-running is idempotent rather than
  # additive -- appending duplicates would silently accumulate on every deploy.
  iptables -N "$CHAIN" 2>/dev/null || true
  iptables -F "$CHAIN"

  iptables -A "$CHAIN" -s 127.0.0.1/32 -j ACCEPT
  for peer in $validated; do
    iptables -A "$CHAIN" -s "$peer" -j ACCEPT
    echo "  allowed: ${peer}"
  done
  iptables -A "$CHAIN" -j DROP

  # A chain whose last rule is not DROP is a chain that accepts by fallthrough.
  # Assert it, rather than trusting that the lines above ran.
  if ! iptables -S "$CHAIN" | tail -1 | grep -q -- "-j DROP"; then
    echo "REFUSING TO CONTINUE: ${CHAIN} does not end in DROP" >&2
    exit 71
  fi

  # Hooked at position 1 so no earlier ACCEPT (Docker's included) can pre-empt
  # it. -C first, so the hook is added once rather than on every run.
  local p
  for p in $PORTS; do
    iptables -C INPUT -p tcp --dport "$p" -j "$CHAIN" 2>/dev/null \
      || iptables -I INPUT 1 -p tcp --dport "$p" -j "$CHAIN"
    echo "  guarded port: ${p}"
  done
  echo "  everything not listed above is DROPped on those ports"
}

persist() {
  need_root
  # This host has neither netfilter-persistent nor /etc/iptables, so rules do
  # NOT survive a reboot on their own. A oneshot unit is the honest fallback --
  # not an assumption that something else saves them.
  cat > "$UNIT" <<UNITEOF
[Unit]
Description=Restrict the qwen38 inference port to federation peers
After=network-online.target docker.service
Wants=network-online.target

[Service]
Type=oneshot
RemainAfterExit=yes
Environment=QWEN38_FED_ALLOW=${QWEN38_FED_ALLOW}
Environment=QWEN38_FED_PORTS=${PORTS}
ExecStart=${HERE}/federation-firewall.sh apply
ExecStop=${HERE}/federation-firewall.sh --remove

[Install]
WantedBy=multi-user.target
UNITEOF
  systemctl daemon-reload
  systemctl enable qwen38-federation-firewall.service >/dev/null
  echo "  boot persistence installed: ${UNIT}"
}

case "$MODE" in
  --status) show ;;
  --remove) remove ;;
  --persist) apply; persist ;;
  apply|--apply) apply ;;
  *) echo "usage: $0 [apply|--persist|--status|--remove]" >&2; exit 64 ;;
esac
