#!/usr/bin/env bash
# Structural tests — no inference, no GPU required. Safe to run in CI.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
pass=0; fail=0
ok()  { printf '  PASS  %s\n' "$*"; pass=$((pass+1)); }
bad() { printf '  FAIL  %s\n' "$*"; fail=$((fail+1)); }
check(){ if [ "$1" = 0 ]; then ok "$2"; else bad "$2"; fi; }

echo "== structural =="

# 1 — every shell artefact parses
for f in "${HERE}/scripts/serve-profile.sh" "${HERE}/scripts/qwen38ctl" \
         "${HERE}/scripts/setup-linux-qwen38.sh" "${HERE}/scripts/measure-ceiling.sh" \
         "${HERE}/scripts/install-node.sh" "${HERE}/scripts/bench-context-sweep.sh" \
         "${HERE}/scripts/measure-slot-frontier.sh" \
         "${HERE}/tests/test_smoke.sh" "${HERE}/tests/test_structural.sh"; do
  bash -n "$f" 2>/dev/null
  check $? "bash -n $(basename "$f")"
done

# 2 — every conf file parses as shell and sets the keys the launcher reads
bash -n "${HERE}/config/common.conf"
check $? "bash -n common.conf"
for f in "${HERE}"/config/profiles.d/*.conf; do
  bash -n "$f" 2>/dev/null && grep -q '^QWEN38_PROFILE_VERSION=' "$f"
  check $? "profile $(basename "$f" .conf) parses and declares a version"
done

# 3 — no profile ships an unresolved measurement placeholder
if grep -rn '__[A-Z_]*__' "${HERE}/config/profiles.d/" >/dev/null 2>&1; then
  bad "profiles still contain unresolved __PLACEHOLDER__ values"
  grep -rn '__[A-Z_]*__' "${HERE}/config/profiles.d/" | sed 's/^/        /'
else
  ok "profiles contain no unresolved placeholders"
fi

# 4 — the unit is valid
unit="${HERE}/systemd/autospec-qwen38@.service"
if command -v systemd-analyze >/dev/null; then
  systemd-analyze verify "$unit" 2>/dev/null
  check $? "systemd-analyze verify autospec-qwen38@.service"
else
  echo "  SKIP  systemd-analyze not available"
fi

# 5 — the unit must not re-introduce the sandbox settings that break CUDA
for bad_setting in 'PrivateDevices=true' 'DevicePolicy=closed' \
                   'MemoryDenyWriteExecute=true' 'ProtectSystem=strict'; do
  ! grep -q "^${bad_setting}" "$unit"
  check $? "unit does not set ${bad_setting} (would break CUDA)"
done

# 6 — the unit runs as the service account, not root
grep -q '^User=qwen-vllm' "$unit"
check $? "unit runs as qwen-vllm"

# 7 — loopback bind by default
grep -q '^QWEN38_HOST="127.0.0.1"' "${HERE}/config/common.conf"
check $? "default bind address is loopback"

# 8 — the vLLM node must not collide with the llama.cpp node's port
vllm_port="$(grep -m1 '^QWEN38_PORT=' "${HERE}/config/common.conf" | cut -d'"' -f2)"
[ "$vllm_port" != "8080" ]
check $? "vLLM port (${vllm_port}) does not collide with llama.cpp on 8080"

# 9 — Conflicts= keeps the two runtimes off the GPU simultaneously
grep -q '^Conflicts=qwen-local.service' "$unit"
check $? "unit conflicts with qwen-local.service"

# 10 — the venv's bin must be on PATH, or FlashInfer's JIT cannot find ninja
for f in "${HERE}/scripts/serve-profile.sh" "${HERE}/scripts/measure-ceiling.sh"; do
  grep -q 'export PATH="${QWEN38_VENV}/bin:${PATH}"' "$f"
  check $? "$(basename "$f") puts the venv bin on PATH (ninja for FlashInfer JIT)"
done

# 10b — every python artefact parses
for f in "${HERE}"/scripts/*.py "${HERE}"/tests/*.py; do
  python3 -c "import ast,sys;ast.parse(open(sys.argv[1]).read())" "$f" 2>/dev/null
  check $? "python -m ast $(basename "$f")"
done

# 11 — the FlashInfer sampler workaround must stay wired to both entry points
grep -q '^QWEN38_USE_FLASHINFER_SAMPLER=' "${HERE}/config/common.conf" \
  && grep -q 'REMOVE WHEN' "${HERE}/config/common.conf"
check $? "FlashInfer sampler workaround is set and carries a removal condition"
for f in "${HERE}/scripts/serve-profile.sh" "${HERE}/scripts/measure-ceiling.sh"; do
  grep -q 'VLLM_USE_FLASHINFER_SAMPLER="\${QWEN38_USE_FLASHINFER_SAMPLER}"' "$f"
  check $? "$(basename "$f") exports the FlashInfer sampler workaround"
done

# 12 — the pinned model is pinned by an immutable revision, not a branch
rev="$(grep -m1 '^QWEN38_MODEL_REVISION=' "${HERE}/config/common.conf" | cut -d'"' -f2)"
printf '%s' "$rev" | grep -Eq '^[0-9a-f]{40}$'
check $? "model revision is a full commit sha (${rev})"

# 13 — the router presets must stay internally consistent.
# The tier aliases ration a shared KV pool that the server does NOT police:
# over-subscribe it and every in-flight session dies with "Context size has
# been exceeded", not just the one that asked for too much. A tier wider than
# its own pool is therefore a live outage waiting to happen.
presets="${HERE}/config/router-presets.ini"
python3 "${HERE}/tests/check_presets.py" "$presets"
check $? "router preset tiers fit their pool, share the KV cache, and have slots"

# 14 — the installer must ship every operator tool.
# A script that exists only in the checkout is missing on any machine
# provisioned from a tarball, and the gap shows up exactly when somebody needs
# the tool. Anything genuinely install-time-only is listed here explicitly, so
# adding a script forces a decision rather than a silent omission.
installer="${HERE}/scripts/install-node.sh"
not_shipped=""
for f in "${HERE}"/scripts/*; do
  b="$(basename "$f")"
  case "$b" in
    # install-time or superseded: these run FROM the checkout, never from bin/
    install-node.sh|setup-linux-qwen38.sh|serve-profile.sh|qwen38ctl) continue ;;
    gen-runtime-descriptor.py|bench-exl3.py) continue ;;
    __pycache__) continue ;;
  esac
  grep -q "$b" "$installer" || not_shipped="${not_shipped} ${b}"
done
if [ -n "$not_shipped" ]; then
  bad "install-node.sh does not ship:${not_shipped}"
else
  ok "install-node.sh ships every operator tool"
fi

echo "== structural: ${pass} passed, ${fail} failed =="
[ "$fail" -eq 0 ]
