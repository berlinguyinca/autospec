#!/usr/bin/env bash
# Issue #4058 — merge driver for generated artifacts (`merge=autospec-regen`).
#
# Git invokes a merge driver with three TEMP files, not paths:
#   %O  base   %A  ours (the driver writes its result here)  %B  theirs
# There is no %f path placeholder, and when the driver runs the worktree and
# index are not fully merged. The driver therefore cannot identify which
# artifact it is resolving or read the sources a regeneration would need, so
# it cannot regenerate. Its one job is to keep git from text-merging the
# artifact: a single-value file (a hash golden) cannot survive a text merge —
# the losing side becomes a second hash, and a build passes over the
# corruption.
#
# Take-theirs is the only correct in-driver behaviour, and it is a
# placeholder: authoritative content comes from the conversion pass, which
# runs the owning generator after the merge (named by the `generated-by`
# attribute in .gitattributes) and stages the result (AC2).
#
# Any failure exits non-zero: git then leaves the file conflicted, which is
# the safe direction — a held conflict is recoverable, a merged corruption is
# not.

set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "merge-regen-driver: expected 3 args (%O %A %B), got $#" >&2
  exit 1
fi

base=$1  # unused: the driver cannot identify the artifact from a temp path
ours=$2
theirs=$3

echo "WARN: generated artifact resolved via take-theirs (issue #4058): this is a placeholder — run the generator named by the generated-by attribute in .gitattributes to refresh" >&2

cat -- "$theirs" > "$ours"
