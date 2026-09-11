#!/usr/bin/env bash
# lint-source-file-share.sh (issue #4066, AC4)
#
# Reports source files that hold more than a configurable share of the
# total source lines under crates/*/src. A single file dominating the
# tree is the condition that made executor_bridge.rs grow to 24,920
# lines; this check makes that condition visible before it recurs.
#
# Usage:
#   lint-source-file-share.sh [--threshold PCT] [--strict] [ROOT]
#
#   --threshold PCT  share of total lines that triggers a report
#                    (default: ${AUTOSPEC_SOURCE_FILE_SHARE_PCT:-10})
#   --strict         exit 1 when any file exceeds the threshold
#                    (default: advisory, always exit 0)
#
# Output: SOURCE_FILE_SHARE:<path>: <lines> lines (<pct>%) of <total>
#         (threshold <threshold>%)
# Exit code: 0 (advisory) or 1 (--strict with findings).
set -eu

SCRIPT_PATH="$0"
SCRIPT_DIR="$(cd "$(dirname "$SCRIPT_PATH")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

THRESHOLD="${AUTOSPEC_SOURCE_FILE_SHARE_PCT:-10}"
STRICT=0

while [ $# -gt 0 ]; do
  case "$1" in
    --threshold) THRESHOLD="${2:?--threshold needs a value}"; shift 2 ;;
    --strict) STRICT=1; shift ;;
    --help|-h)
      sed -n '2,20p' "$SCRIPT_PATH"
      exit 0
      ;;
    -*)
      echo "unknown option: $1" >&2
      exit 2
      ;;
    *) ROOT_DIR="$(cd "$1" && pwd)"; shift ;;
  esac
done

case "$THRESHOLD" in
  ''|*[!0-9.]*)
    echo "error: threshold must be a number, got '$THRESHOLD'" >&2
    exit 2
    ;;
esac

# shellcheck disable=SC2086
python3 - "$ROOT_DIR" "$THRESHOLD" "$STRICT" <<'PY'
import sys, os

root, threshold_s, strict = sys.argv[1], sys.argv[2], sys.argv[3] == "1"
threshold = float(threshold_s)

files = []
for crate_dir in sorted(os.listdir(os.path.join(root, "crates"))):
    src = os.path.join(root, "crates", crate_dir, "src")
    if not os.path.isdir(src):
        continue
    for dirpath, _dirnames, filenames in os.walk(src):
        for name in sorted(filenames):
            if name.endswith(".rs"):
                files.append(os.path.join(dirpath, name))

total = 0
for f in files:
    with open(f, "rb") as fh:
        total += sum(1 for _ in fh)

if total == 0:
    print("SOURCE_FILE_SHARE: no source files found under crates/*/src")
    sys.exit(0)

findings = []
for f in files:
    with open(f, "rb") as fh:
        n = sum(1 for _ in fh)
    pct = 100.0 * n / total
    if pct > threshold:
        rel = os.path.relpath(f, root)
        findings.append((pct, rel, n))

for pct, rel, n in sorted(findings, reverse=True):
    print(f"SOURCE_FILE_SHARE:{rel}: {n} lines ({pct:.1f}%) of {total} "
          f"(threshold {threshold:g}%)")

if strict and findings:
    sys.exit(1)
sys.exit(0)
PY
