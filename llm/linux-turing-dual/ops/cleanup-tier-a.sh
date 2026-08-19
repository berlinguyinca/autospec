#!/usr/bin/env bash
# Tier A host cleanup: reversible by reinstalling a package.
#
#   cleanup-tier-a.sh              dry run (default -- prints, changes nothing)
#   cleanup-tier-a.sh --apply      actually do it
#
# Dry run is the default because the destructive form of this script is one typo
# away from removing the running kernel.
#
# KERNEL POLICY, and it is load-bearing:
#   6.8.0-111  is RUNNING
#   6.8.0-138  is the NEXT BOOT TARGET and has never booted on this host
#   6.8.0-101  is the FALLBACK, retired only after 138 boots with working GPUs
# This script refuses to touch any of the three, AND marks them manual before
# running autoremove -- refusing to purge them is not enough, because
# autoremove will take an auto-installed kernel that nothing depends on. That
# is not hypothetical: the first run of this script lost 6.8.0-101 that way.
# Removing a fallback before its replacement has proven itself is how a remote
# host becomes a site visit.
set -euo pipefail

APPLY=0
case "${1:-}" in
  --apply)   APPLY=1 ;;
  --dry-run|"") APPLY=0 ;;
  *) echo "usage: $(basename "$0") [--apply|--dry-run]" >&2; exit 64 ;;
esac

run() {
  if [ "$APPLY" -eq 1 ]; then
    echo "+ $*"
    "$@"
  else
    echo "would run: $*"
  fi
}

say() { printf '\n=== %s\n' "$*"; }

# --- kernels we must never remove ------------------------------------------
PROTECTED_KERNELS="6.8.0-111 6.8.0-138 6.8.0-101"
RUNNING="$(uname -r)"

# Kernels to remove, discovered rather than hardcoded, then filtered.
mapfile -t OLD_KERNELS < <(
  dpkg -l 2>/dev/null | awk '/^ii  linux-(image|modules|modules-extra|headers)-[0-9]/ {print $2}' | sort -u
)
REMOVE_KERNEL_PKGS=()
for pkg in "${OLD_KERNELS[@]:-}"; do
  keep=0
  for prot in $PROTECTED_KERNELS; do
    case "$pkg" in *"$prot"*) keep=1 ;; esac
  done
  # Belt and braces: never the running kernel, whatever the list says.
  case "$pkg" in *"${RUNNING%-generic}"*) keep=1 ;; esac
  [ "$keep" -eq 0 ] && REMOVE_KERNEL_PKGS+=("$pkg")
done

say "kernel policy"
echo "running:   ${RUNNING}"
echo "protected: ${PROTECTED_KERNELS}"
echo "to remove: ${REMOVE_KERNEL_PKGS[*]:-<none>}"
for pkg in "${REMOVE_KERNEL_PKGS[@]:-}"; do
  for prot in $PROTECTED_KERNELS; do
    case "$pkg" in *"$prot"*)
      echo "REFUSING: ${pkg} matches protected kernel ${prot}" >&2; exit 1 ;;
    esac
  done
done

# --- protect the kernels from AUTOREMOVE, not just from the purge list ------
# Learned the hard way on the first run: the explicit purge honoured the
# protected list, and then `apt-get autoremove --purge` removed 6.8.0-101
# anyway, because an auto-installed kernel with nothing depending on it is
# exactly what autoremove is for. Marking them manual is what actually holds.
say "pin protected kernels against autoremove"
for prot in $PROTECTED_KERNELS; do
  for kind in image modules modules-extra headers; do
    pkg="linux-${kind}-${prot}-generic"
    if dpkg -l "$pkg" 2>/dev/null | grep -q '^ii'; then
      run sudo apt-mark manual "$pkg"
    fi
  done
done

say "disk before"
df -h / | tail -1

# --- 1. nvidia: swap the X-pulling metapackage for the headless one --------
# Order matters. Installing headless in the SAME transaction that removes the
# -driver- metapackage lets apt resolve it as a swap, instead of leaving a gap
# where the DKMS module used to be.
say "nvidia: install headless, then drop the X-pulling driver metapackage"
run sudo apt-get install -y --no-install-recommends nvidia-headless-580 nvidia-utils-580

# --- 2. the desktop stack --------------------------------------------------
DESKTOP_PKGS=(
  ubuntu-desktop ubuntu-desktop-minimal
  gnome-shell gnome-shell-common
  gnome-shell-extension-appindicator
  gnome-shell-extension-desktop-icons-ng
  gnome-shell-extension-ubuntu-dock
  gnome-shell-extension-ubuntu-tiling-assistant
  gdm3 xserver-xorg-core
  thunderbird cups-daemon
  davinci-resolve jenkins
)
say "desktop stack"
run sudo apt-get purge -y "${DESKTOP_PKGS[@]}" 'libreoffice-style-*' 'nvidia-driver-580'

# --- 3. driver sediment ---------------------------------------------------
SEDIMENT=(
  libnvidia-compute-390 libnvidia-compute-418 libnvidia-compute-430
  libnvidia-compute-470 libnvidia-compute-525 libnvidia-compute-535
  nvidia-dkms-435 nvidia-dkms-525 nvidia-dkms-535
  nvidia-driver-525 nvidia-driver-535 nvidia-utils-535
  cuda-toolkit-10-0
)
say "driver sediment (390/418/430/470/525/535 + cuda 10)"
run sudo apt-get purge -y "${SEDIMENT[@]}"

# --- 4. old kernels -------------------------------------------------------
say "old kernels"
if [ "${#REMOVE_KERNEL_PKGS[@]}" -gt 0 ]; then
  run sudo apt-get purge -y "${REMOVE_KERNEL_PKGS[@]}"
else
  echo "nothing to remove"
fi

# --- 5. desktop snaps -----------------------------------------------------
# gh is a snap and is KEPT. Bases (core*, bare) are kept: snapd needs them and
# snapd itself stays, because removing it would take gh with it.
say "desktop snaps (gh is kept deliberately)"
for s in chromium firefox cups dbeaver-ce gthumb \
         gnome-3-26-1604 gnome-3-28-1804 gnome-3-34-1804 gnome-3-38-2004 \
         gnome-42-2204 gnome-46-2404; do
  if snap list "$s" >/dev/null 2>&1; then
    run sudo snap remove --purge "$s"
  else
    echo "not installed: $s"
  fi
done

# --- 6. /opt relics -------------------------------------------------------
say "/opt relics"
for d in resolve idea-IU-181.4203.550 idea-IC-173.4548.28 pycharm-2017.3.4 \
         ideaIU-2018.1.tar.gz WebStorm-181.4203.535 monero-gui-v0.11.1.0; do
  if [ -e "/opt/${d}" ]; then
    run sudo rm -rf "/opt/${d}"
  else
    echo "absent: /opt/${d}"
  fi
done

# --- 7. logs, caches, obsolete tools -------------------------------------
say "journal, caches, obsolete mlocate (plocate is already installed)"
run sudo journalctl --vacuum-size=200M
if dpkg -l mlocate 2>/dev/null | grep -q '^ii'; then
  run sudo apt-get purge -y mlocate
fi
run sudo apt-get autoremove --purge -y
run sudo apt-get clean

say "disk after"
df -h / | tail -1

if [ "$APPLY" -eq 0 ]; then
  printf '\nDRY RUN -- nothing was changed. Re-run with --apply.\n'
fi
