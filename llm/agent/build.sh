#!/usr/bin/env bash
# Cross-build the agent for every platform a GPU box might be.
#
# One host builds all of them because the agent is pure Go: CGO_ENABLED=0 means no
# C toolchain and no per-platform build machine. If this ever needs one, something
# has grown a dependency and that is the thing to fix.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$HERE"
out="${1:-dist}"
mkdir -p "$out"

# The zero-dependency property, asserted rather than hoped for: `go list -m all`
# prints the main module and nothing else.
deps="$(go list -m all | tail -n +2 || true)"
if [ -n "$deps" ]; then
  echo "FAIL -- the agent has grown dependencies:" >&2
  echo "$deps" >&2
  exit 1
fi

for t in linux/amd64 linux/arm64 windows/amd64 darwin/arm64 darwin/amd64; do
  os="${t%/*}"; arch="${t#*/}"
  name="qwen-turing-agent-${os}-${arch}"
  [ "$os" = windows ] && name="${name}.exe"
  CGO_ENABLED=0 GOOS="$os" GOARCH="$arch" \
    go build -trimpath -ldflags="-s -w" -o "${out}/${name}" .
  printf '%-40s %6s KB\n' "$name" "$(( $(stat -c%s "${out}/${name}") / 1024 ))"
done
echo "OK -- five targets, no C toolchain, no dependencies"
