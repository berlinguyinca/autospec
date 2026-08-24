#!/usr/bin/env bash
# Firewall for the dual-Turing node.
#
#   firewall.sh                 print what it would do
#   firewall.sh --apply         apply it
#
# ONE public port. nginx owns 80 (and 8080 for clients configured before the
# move); llama.cpp and the dashboard bind loopback only, so there is nothing to
# open for them and 8081 is deliberately CLOSED.
#
# Ranges come from the environment so this file carries no site identifiers --
# the repository is public:
#   QT_FW_CAMPUS    the range that may reach inference   (e.g. 192.0.2.0/24)
#   QT_FW_INTERNAL  the internal LAN
#   QT_FW_MGMT      where operators SSH from
set -euo pipefail

APPLY=0
case "${1:-}" in
  --apply) APPLY=1 ;;
  ""|--dry-run) APPLY=0 ;;
  *) echo "usage: $(basename "$0") [--apply]" >&2; exit 64 ;;
esac

: "${QT_FW_CAMPUS:?set QT_FW_CAMPUS (CIDR allowed to reach inference)}"
: "${QT_FW_INTERNAL:?set QT_FW_INTERNAL (internal LAN CIDR)}"
: "${QT_FW_MGMT:?set QT_FW_MGMT (CIDR operators SSH from)}"

run() { if [ "$APPLY" -eq 1 ]; then echo "+ $*"; "$@"; else echo "would run: $*"; fi; }

# DEAD-MAN SWITCH. Enabling default-deny over SSH is how a host becomes
# unreachable; this un-does it if the rules are wrong and nobody can log in to
# fix them. Cancel it only after a SECOND, independent session has proven SSH.
if [ "$APPLY" -eq 1 ]; then
  sudo bash -c 'nohup sh -c "sleep 900; ufw --force disable; logger qt-fw: auto-disabled" >/dev/null 2>&1 &'
  echo "dead-man switch armed: ufw auto-disables in 15 minutes"
fi

# Docker manipulates FORWARD directly, and this host also carries VLAN bridges and
# libvirt networks. ufw must not take that chain over.
run sudo sed -i 's/^DEFAULT_FORWARD_POLICY=.*/DEFAULT_FORWARD_POLICY="ACCEPT"/' /etc/default/ufw

run sudo ufw --force reset
run sudo ufw default deny incoming
run sudo ufw default allow outgoing

# SSH first, and from BOTH operator ranges: losing this rule loses the host.
run sudo ufw allow from "${QT_FW_INTERNAL}" to any port 22 proto tcp
run sudo ufw allow from "${QT_FW_MGMT}"     to any port 22 proto tcp

# The one public service port, plus the compatibility port.
for range in "${QT_FW_CAMPUS}" "${QT_FW_INTERNAL}" "${QT_FW_MGMT}"; do
  run sudo ufw allow from "$range" to any port 80   proto tcp
  run sudo ufw allow from "$range" to any port 8080 proto tcp
  # 443 only once a certificate exists -- a rule for a port nothing listens on
  # is a rule nobody will remember to review. Scoped to the SAME ranges as 80,
  # deliberately: a global allow would make the dashboard and the login flow
  # reachable from further away than the inference endpoint they belong to.
  if [ -r /etc/ssl/qwen-turing/fullchain.pem ]; then
    run sudo ufw allow from "$range" to any port 443 proto tcp
  fi
done

# NOT opened: 8081 (dashboard) and 8090 (llama.cpp) bind loopback and are reached
# through nginx.

run sudo ufw --force enable
[ "$APPLY" -eq 1 ] && sudo ufw status verbose

cat <<'NOTE'

Before cancelling the dead-man switch, open a SECOND independent SSH session and
confirm it works. Then:
    sudo pkill -f 'sleep 900'
    sudo systemctl enable ufw
NOTE
