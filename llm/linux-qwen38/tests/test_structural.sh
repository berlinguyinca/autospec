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

# 15 — every cluster artefact parses
for f in "${HERE}"/slurm/*.sh "${HERE}"/slurm/opencode_slurm "${HERE}"/slurm/*.sbatch; do
  bash -n "$f" 2>/dev/null
  check $? "bash -n slurm/$(basename "$f")"
done
for f in "${HERE}"/slurm/*.py; do
  python3 -c "import ast,sys;ast.parse(open(sys.argv[1]).read())" "$f" 2>/dev/null
  check $? "python -m ast slurm/$(basename "$f")"
done

# 16 — under `set -e` with pipefail, an unguarded substitution containing a pipe
# kills the script with no message when a remote helper legitimately fails while
# polling. This exact bug stopped the driver dead twice, reported only as a bare
# `exit 2`, and it is invisible on inspection.
if grep -nE '="\$\(.*\|' "${HERE}/slurm/opencode_slurm" | grep -qv '|| true'; then
  bad "slurm/opencode_slurm has a pipe in a command substitution without || true"
  grep -nE '="\$\(.*\|' "${HERE}/slurm/opencode_slurm" | grep -v '|| true' | sed 's/^/        /'
else
  ok "slurm/opencode_slurm guards every piped command substitution"
fi

# 17 — a preset generated for any supported card must satisfy the same
# invariants the static ones do, or the client is told about context the pool
# cannot fund. 24564 stands for a card too small, which must produce nothing.
gp="${HERE}/slurm/gen-preset.py"
gen_ok=0
for vram in 97887 81920 46068 40960; do
  tmp="$(mktemp)"
  python3 "$gp" --vram-mib "$vram" --models-dir /m >"$tmp" 2>/dev/null
  if ! grep -q '^c = ' "$tmp"; then
    bad "gen-preset.py produced no model for ${vram} MiB"
    gen_ok=1
  elif ! python3 "${HERE}/tests/check_presets.py" "$tmp" >/dev/null 2>&1; then
    bad "gen-preset.py output for ${vram} MiB fails the preset invariants"
    python3 "${HERE}/tests/check_presets.py" "$tmp" | sed 's/^/        /'
    gen_ok=1
  fi
  rm -f "$tmp"
done
check $gen_ok "gen-preset.py output is valid for every supported card"

# A card too small for the weights must refuse, not emit something unservable.
tmp="$(mktemp)"
python3 "$gp" --vram-mib 24564 --models-dir /m >"$tmp" 2>/dev/null
! grep -q '^c = ' "$tmp"
check $? "gen-preset.py refuses a card too small for the weights"
rm -f "$tmp"

# 18 — the client must be configured from what the server publishes, never from
# a template written for the largest card. Advertising a model the server does
# not have is a 400; advertising more context than the pool funds kills every
# live session, not just the greedy one.
grep -q 'router-presets.active.ini' "${HERE}/slurm/opencode_slurm"
check $? "opencode_slurm configures from the job's published active preset"
! grep -q 'router-presets-cluster.ini' "${HERE}/slurm/opencode_slurm"
check $? "opencode_slurm does not read the per-card-agnostic template"

# 19 — the GPU registry must stay machine-readable and honest about what is
# measured. A prediction for a machine nobody has touched rests on it.
python3 - "${HERE}/config/gpu-registry.json" <<'PYCHK'
import json, sys
reg = json.load(open(sys.argv[1]))
bad = []
if not 0.5 <= reg.get("roofline_efficiency", 0) <= 1.0:
    bad.append("roofline_efficiency is not a sane fraction")
for name, g in reg.get("gpus", {}).items():
    if not isinstance(g.get("vram_mib"), int) or g["vram_mib"] <= 0:
        bad.append(f"{name}: vram_mib must be a positive int")
    bw = g.get("bandwidth_gbs")
    if bw is not None and not (100 <= bw <= 10000):
        bad.append(f"{name}: bandwidth_gbs {bw} is out of range")
    for quant, tps in (g.get("measured_tps") or {}).items():
        if not isinstance(tps, (int, float)) or tps <= 0:
            bad.append(f"{name}: measured_tps[{quant}] must be a positive number")
print("\n".join(f"        {b}" for b in bad))
sys.exit(1 if bad else 0)
PYCHK
check $? "gpu-registry.json is well-formed"

# 20 — the python suites here are scripts, not pytest modules, and they exit at
# import time. Left collectable, pytest reports INTERNALERROR, which reads like
# a broken test rather than a misuse of the runner.
conftest="${HERE}/tests/conftest.py"
[ -f "$conftest" ] && grep -q 'collect_ignore_glob' "$conftest"
check $? "conftest keeps pytest out of the standalone script suites"

# 21 — the reconnect counter is what an operator reads to decide whether the
# tunnel has been stable. Seeded from zero it forgets every restart of the
# supervisor itself and reports a quiet night that did not happen.
grep -q 'reconnects="\$(cat "\${STATE}/tunnel.reconnects"' "${HERE}/slurm/tunnel-supervisor.sh"
check $? "the reconnect counter survives a supervisor restart"

# 22 — this repository is public, so no real site identifier may be committed.
# The values live in a local site.conf; the placeholders here are what a reader
# is supposed to see. Scoped to the trees this project owns, and one pattern per
# check so a leak names itself.
root="$(cd "${HERE}/../.." && pwd)"
leaked=""
# This file is excluded because it necessarily contains every pattern it looks
# for; without that it reports itself as the leak. Everything else is in scope.
self=':!llm/linux-qwen38/tests/test_structural.sh'
# 'ucdavis' alone missed "UC Davis" with a space, which is how three references
# survived the first scrub -- hence the separator class.
for pat in 'ucdavis' 'uc[ -]davis' 'metabolomicsgrp' 'publicgrp' '/quobyte'; do
  if git -C "$root" grep -qiIE "$pat" -- llm docs/memory "$self" 2>/dev/null; then
    leaked="${leaked} ${pat}"
  fi
done
if [ -n "$leaked" ]; then
  bad "a real site identifier is committed:${leaked}"
  for pat in $leaked; do
    git -C "$root" grep -niIE "$pat" -- llm docs/memory "$self" 2>/dev/null | sed 's/^/        /' | head -3
  done
else
  ok "no real site identifier is committed"
fi

# 23 — the example must actually cover what require_site() demands, or the
# message tells you to copy a file that does not answer it.
ex="${HERE}/slurm/site.conf.example"
missing_ex=""
for v in OPENCODE_SLURM_HOST OPENCODE_SLURM_USER OPENCODE_SLURM_ROOT OPENCODE_SLURM_ACCOUNT; do
  grep -q "^: \"\${${v}:=" "$ex" 2>/dev/null || missing_ex="${missing_ex} ${v}"
done
if [ -n "$missing_ex" ]; then
  bad "site.conf.example does not set:${missing_ex}"
else
  ok "site.conf.example covers every value require_site() demands"
fi

# 24 — an unconfigured driver must fail with EX_CONFIG, not wander off to ssh.
OPENCODE_SLURM_SITE_CONF=/nonexistent "${HERE}/slurm/opencode_slurm" status >/dev/null 2>&1
[ "$?" = 78 ]
check $? "opencode_slurm refuses to run unconfigured (EX_CONFIG)"

# 25 — the tooling is named for the scheduler, not for one institution's cluster.
# Kept separate from the site-identifier check above because it is a naming
# concern, not a privacy one, and unlike that one it has a legitimate exception:
# adopting a pre-rename state directory requires naming it. Patterns are anchored
# so they cannot match "archive".
allow='leak-guard-allow'
stale=""
for pat in '\bhive\b' 'hive[-_]' '[-_]hive'; do
  hits="$(git -C "$root" grep -niIE "$pat" -- llm docs/memory "$self" 2>/dev/null | grep -v "$allow" || true)"
  if [ -n "$hits" ]; then stale="${stale} ${pat}"; fi
done
if [ -n "$stale" ]; then
  bad "a pre-rename 'hive' reference remains:${stale}"
  for pat in $stale; do
    git -C "$root" grep -niIE "$pat" -- llm docs/memory "$self" 2>/dev/null \
      | grep -v "$allow" | sed 's/^/        /' | head -3
  done
else
  ok "no pre-rename 'hive' reference remains"
fi

# 26 — within-slot prefix reuse must be on in every launch path. Subagents share
# a large identical preamble; reprocessing it per request is the one waste here
# that costs nothing to avoid. Distinct from the cross-slot selection that is
# deliberately OFF -- if a future edit conflates them, this check still passes
# while the crash mitigation is silently lost, so both are asserted.
grep -q '^cache-reuse = ' "${HERE}/config/router-presets.ini" \
  && grep -q 'cache-reuse' "${HERE}/slurm/gen-preset.py" \
  && grep -q 'cache-reuse' "${HERE}/scripts/serve-profile.sh"
check $? "prefix reuse is enabled in the preset, the generator and the launcher"

grep -q '^slot-prompt-similarity = 0.0' "${HERE}/config/router-presets.ini"
check $? "cross-slot selection stays disabled (the crash mitigation)"

echo "== structural: ${pass} passed, ${fail} failed =="
[ "$fail" -eq 0 ]
