#!/usr/bin/env bash
# Tier B (moves) and Tier C (operator-authorised deletions).
#
#   cleanup-tier-bc.sh              dry run (default)
#   cleanup-tier-bc.sh --apply      actually do it
#
# Tier B MOVES rather than deletes: reclaiming the root filesystem should not
# require destroying anything.
#
# Tier C is destructive and was authorised item by item. It carries BARRIER 4:
# /home/leon's only other copy lives on the bulk array, so the array's health
# and the backup's readability are checked INSIDE this script, immediately
# before the delete -- not inherited from a check made an hour ago in another
# phase. Failing either check skips the account removal rather than proceeding.
set -euo pipefail

APPLY=0
case "${1:-}" in
  --apply) APPLY=1 ;;
  --dry-run|"") APPLY=0 ;;
  *) echo "usage: $(basename "$0") [--apply|--dry-run]" >&2; exit 64 ;;
esac

BULK="${QT_BULK_ARRAY:?set QT_BULK_ARRAY to the bulk array mountpoint}"
NVME="${QT_NVME_ARRAY:?set QT_NVME_ARRAY to the fast array mountpoint}"
HOME_DIR="${QT_HOME:-$HOME}"

run() { if [ "$APPLY" -eq 1 ]; then echo "+ $*"; "$@"; else echo "would run: $*"; fi; }
say() { printf '\n=== %s\n' "$*"; }

say "disk before"
df -h / "$BULK" "$NVME" | tail -4

# ---------------------------------------------------------------- Tier B ----
say "Tier B.1 -- libvirt images to the bulk array (domains stay defined)"
# /var/lib/libvirt/images is root-only (0600 files, root:root). An unprivileged
# glob here silently finds nothing, which would have skipped 36 GiB and reported
# success -- so the enumeration runs under sudo.
mapfile -t QCOWS < <(sudo find /var/lib/libvirt/images -maxdepth 1 -name '*.qcow2' -print 2>/dev/null | sort)
if [ "${#QCOWS[@]}" -gt 0 ]; then
  echo "found ${#QCOWS[@]} image(s):"
  for img in "${QCOWS[@]}"; do
    sudo du -h "$img" 2>/dev/null | sed 's/^/    /'
  done
  run sudo mkdir -p "${BULK}/archive/libvirt-images"
  for img in "${QCOWS[@]}"; do
    run sudo mv -v "$img" "${BULK}/archive/libvirt-images/"
  done
else
  echo "no qcow2 images present"
fi

say "Tier B.2 -- remove LM Studio entirely (clean slate, operator request)"
# NOT relocated. The operator chose a clean slate rather than inheriting an
# LM Studio model store, so the weights this node serves are downloaded fresh
# against pinned revisions instead of adopting whatever happened to be on disk.
#
# That is the better provenance anyway: the store held a Qwen3.5-9B-Q4_K_M from
# a different uploader than the one model-artifacts.yaml pins, and "a file with
# the right name" is not an identity.
for p in "${NVME}/lmstudio" "${HOME_DIR}/.lmstudio" "${HOME_DIR}/.lmstudio-home-pointer"; do
  if [ -e "$p" ] || [ -L "$p" ]; then
    run sudo rm -rf "$p"
  else
    echo "absent: $p"
  fi
done

say "Tier B.3 -- stale JetBrains caches"
for d in .PyCharm2018.1 .PyCharm2019.1 .PyCharm2019.3 .IntelliJIdea2018.1; do
  if [ -d "${HOME_DIR}/${d}" ]; then
    run rm -rf "${HOME_DIR}/${d}"
  else
    echo "absent: ${d}"
  fi
done

# ---------------------------------------------------------------- Tier C ----
say "BARRIER 4 -- array health and backup readability, checked HERE"

barrier_ok=1

if grep -qE '\[[U_]*_[U_]*\]' /proc/mdstat; then
  echo "REFUSING: an md array is degraded:" >&2
  grep -E '^\S+ : |\[[U_]+\]' /proc/mdstat >&2
  barrier_ok=0
else
  echo "ok -- no md array reports a missing member"
fi

if sudo test -r "${BULK}/backup/leon"; then
  sz="$(sudo du -sh "${BULK}/backup/leon" 2>/dev/null | cut -f1)"
  echo "ok -- ${BULK}/backup/leon is readable (${sz})"
else
  echo "REFUSING: ${BULK}/backup/leon is not readable -- it is /home/leon's only other copy" >&2
  barrier_ok=0
fi

if [ "$barrier_ok" -eq 1 ]; then
  say "Tier C.1 -- remove the four authorised accounts"
  # Verified beforehand: no running processes, no crontabs, no files outside
  # $HOME, never logged in. The bulk-array backup of leon is PRESERVED.
  for u in jvogel diego sajjan; do
    if getent passwd "$u" >/dev/null; then
      run sudo userdel -r "$u"
    else
      echo "absent: $u"
    fi
  done
  if getent passwd leon >/dev/null; then
    run sudo gpasswd -d leon docker
    run sudo userdel -r leon
  else
    echo "absent: leon"
  fi
else
  say "Tier C.1 -- SKIPPED: barrier 4 failed"
fi

say "Tier C.2 -- prune Docker images and stopped containers"
# After the accounts on purpose: if the arrays are unhappy, that surfaces on the
# cheaper operation first. Docker's root is on the bulk array, so this frees the
# array and not the root filesystem.
#
# Volumes are NOT pruned. 64 of them hold data and one is Postgres-shaped;
# volumes are where databases live and are explicitly out of scope.
if command -v docker >/dev/null 2>&1; then
  for svc in denmark_binbase denmark_binview denmark_node; do
    docker service inspect "$svc" >/dev/null 2>&1 && run docker service rm "$svc" || echo "no service: $svc"
  done
  for c in wcmc_postgresql13_1 binbase.steinlee; do
    docker container inspect "$c" >/dev/null 2>&1 && run docker rm "$c" || echo "no container: $c"
  done
  run docker image prune -a -f
  run docker builder prune -a -f
  echo "--- NOT pruning volumes, deliberately ---"
  docker volume ls -q 2>/dev/null | wc -l | sed 's/^/  volumes left alone: /'
else
  echo "docker absent"
fi

say "disk after"
df -h / "$BULK" "$NVME" | tail -4
docker system df 2>/dev/null || true

[ "$APPLY" -eq 0 ] && printf '\nDRY RUN -- nothing was changed. Re-run with --apply.\n'
exit 0
