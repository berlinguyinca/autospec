# Dual-Turing Qwen Inference Node Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn an old dual-RTX-2080-Ti Docker/compute host into a clean, headless, API-key-protected Qwen inference node that switches models on demand and reports its own throughput.

**Architecture:** A new `llm/linux-turing-dual/` node directory reusing the existing `llm/linux-qwen38/scripts/` toolkit rather than forking it. llama.cpp in router mode serves Qwen3.8-27B UD-Q4_K_M layer-split across both cards plus Qwen3.5-9B pinned to one card, with `--models-max 1` so a model switch is a reload. Two multi-GPU bugs in the shared toolkit are fixed first, because both silently break on a two-card host. Host cleanup and the driver purge precede a hard reboot barrier; the firewall and all measurement follow it.

**Tech Stack:** bash, Python 3.12 + pytest, llama.cpp (CUDA `sm_75`), systemd, ufw, Ubuntu 24.04.

**Spec:** [`docs/superpowers/specs/2026-08-19-turing-dual-qwen-node-design.md`](../specs/2026-08-19-turing-dual-qwen-node-design.md)

## Global Constraints

- **This repository is public.** No hostname, IP address, subnet or campus identifier may be committed under `llm/` or `docs/memory/`. Real values live in `${XDG_CONFIG_HOME:-~/.config}/qwen-turing/site.conf` (mode 600). Placeholders in committed files: `<node-addr>`, `<node-host>`, `<uplink-if>`, `<nvme-array>`, `<bulk-array>`.
- **Never configure a number you have not verified with a real request.** "Allocates", "starts" and "works at length" are three different claims.
- **Compute capability is `7.5` (sm_75).** No bf16, no FP8, no NVFP4, no Marlin. Build llama.cpp with `-DCMAKE_CUDA_ARCHITECTURES=75`.
- **systemd sandboxing ceiling on a CUDA host:** `ProtectSystem=full` — never `strict`. Never set `PrivateDevices=true`, `DevicePolicy=closed`, or `MemoryDenyWriteExecute=`. Each one breaks CUDA. Reasons must stay as comments in the unit.
- **Weights live on `<nvme-array>`, never on `/`** (`/` has ~55 GiB free before cleanup).
- **Compute buffers are paid per card**, not once, in every VRAM calculation.
- **Aggregate VRAM is 22.0 GiB** (2 × 11.0 GiB). Per-card ceiling is 11.0 GiB for anything pinned to one card.
- **KV cost is derived from `config.json`, not guessed:** `bytes/token = 2 × N_full_attention_layers × kv_heads × head_dim`. 27B = 16 of 64 full-attn → 18.0 KiB/token at `q4_0`. 9B = 8 of 32 full-attn → 9.0 KiB/token at `q4_0`.
- **Cross-slot KV reuse (`slot-prompt-similarity`) stays `0.0`.** Within-slot `cache-reuse` is on. Do not "improve" this.
- **The reboot is a barrier.** Nothing after it may be attempted before it. Nothing destructive that can wait until after a confirmed-good boot may run before it.
- **Commit shaping:** every source commit needs its own doc touch; keep commits under ~400 lines.

---

## File Structure

**New — the node directory:**

| path | responsibility |
|---|---|
| `llm/linux-turing-dual/README.md` | the build as measured; the operator's entry point |
| `llm/linux-turing-dual/config/common.conf` | single source of truth for paths, ports, pins, VRAM guard |
| `llm/linux-turing-dual/config/site.conf.example` | every value the loader demands, with placeholders |
| `llm/linux-turing-dual/config/profiles.d/router.conf` | the one profile that runs; router mode |
| `llm/linux-turing-dual/config/router-presets.ini` | served model ids, pools, seats, tensor-split |
| `llm/linux-turing-dual/config/model-artifacts.yaml` | immutable revision pins for both models |
| `llm/linux-turing-dual/scripts/site.sh` | loads + validates site.conf; exits 78 on placeholder |
| `llm/linux-turing-dual/scripts/install-node.sh` | provisions, then *verifies* with a real completion |
| `llm/linux-turing-dual/scripts/vram-guard.sh` | multi-GPU free-VRAM check used by the launcher |
| `llm/linux-turing-dual/scripts/collect-stats.py` | polls `/metrics` + `nvidia-smi`, emits JSON |
| `llm/linux-turing-dual/scripts/dashboard.py` | serves the stats page; single listener, API-key gated |
| `llm/linux-turing-dual/web/index.html` | the page itself; no external assets |
| `llm/linux-turing-dual/systemd/qwen-turing@.service` | the inference unit |
| `llm/linux-turing-dual/systemd/qwen-turing-dashboard.service` | the stats unit |
| `llm/linux-turing-dual/tests/test_structural.sh` | no-accelerator checks incl. the leak guard |
| `llm/linux-turing-dual/tests/test_site.py` | site.conf loader behaviour |
| `llm/linux-turing-dual/tests/test_vram_guard.py` | multi-GPU guard arithmetic |
| `llm/linux-turing-dual/docs/measured-ceilings.md` | what was measured, what was rejected |
| `llm/linux-turing-dual/ops/cleanup-tier-a.sh` | reversible host cleanup |
| `llm/linux-turing-dual/ops/cleanup-tier-bc.sh` | moves + authorised deletions, with barriers |
| `llm/linux-turing-dual/ops/prep-headless.sh` | driver purge, headless swap, DKMS re-verify |

**Modified — shared toolkit (fixes benefit both nodes):**

| path | change |
|---|---|
| `llm/linux-qwen38/scripts/select-quant.py` | enumerate all GPUs, not `.splitlines()[0]` |
| `llm/linux-qwen38/scripts/serve-profile.sh` | multi-GPU VRAM guard |
| `llm/linux-qwen38/config/gpu-registry.json` | add RTX 2080 Ti |
| `llm/README.md` | fourth row for the new node |
| `.github/workflows/python.yml` | run both nodes' structural tests |

---

## Phase A — pre-reboot. Everything here is reversible.

### Task 1: Multi-GPU device enumeration in `select-quant.py`

The tool answers "which quantisation fits this hardware". It reads
`nvidia-smi --query-gpu=memory.total` and takes `.splitlines()[0]`, so on this
host it sees 11 GiB and concludes nothing fits.

**Files:**
- Modify: `llm/linux-qwen38/scripts/select-quant.py` (the `nvidia vram` branch, ~line 105-115)
- Test: `llm/linux-qwen38/tests/test_select_quant_devices.py`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `detect_devices() -> list[int]` returning per-device total MiB, and
  `device_budget(devices: list[int], reserve_per_card_mib: int) -> tuple[int, str]`
  returning `(usable_mib, explanation)`. Task 2 and Task 5 both read these.

- [ ] **Step 1: Write the failing test**

```python
# llm/linux-qwen38/tests/test_select_quant_devices.py
import importlib.util, pathlib, pytest

SRC = pathlib.Path(__file__).resolve().parents[1] / "scripts" / "select-quant.py"

def load():
    spec = importlib.util.spec_from_file_location("select_quant", SRC)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod

def test_parses_every_device_not_just_the_first():
    m = load()
    assert m.parse_device_totals("11264\n11264\n") == [11264, 11264]

def test_single_device_still_works():
    m = load()
    assert m.parse_device_totals("24564\n") == [24564]

def test_blank_lines_ignored():
    m = load()
    assert m.parse_device_totals("11264\n\n11264\n\n") == [11264, 11264]

def test_budget_charges_per_card_reserve():
    m = load()
    # two cards, 512 MiB reserved on EACH -- not once
    usable, why = m.device_budget([11264, 11264], reserve_per_card_mib=512)
    assert usable == 11264 * 2 - 512 * 2
    assert "2 devices" in why

def test_budget_reports_per_card_ceiling_for_pinned_models():
    m = load()
    assert m.per_card_ceiling([11264, 11264], reserve_per_card_mib=512) == 11264 - 512
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-qwen38/tests/test_select_quant_devices.py -v`
Expected: FAIL — `AttributeError: module 'select_quant' has no attribute 'parse_device_totals'`

- [ ] **Step 3: Write minimal implementation**

Add these three functions near the existing detection helper, then make the
`nvidia vram` branch call them:

```python
def parse_device_totals(raw: str) -> list[int]:
    """Every device nvidia-smi reported, in MiB. The first line is not the answer."""
    return [int(line.strip()) for line in raw.splitlines() if line.strip()]


def device_budget(devices: list[int], reserve_per_card_mib: int = 512) -> tuple[int, str]:
    """Aggregate usable MiB. Compute buffers and CUDA context are paid PER CARD."""
    usable = sum(devices) - reserve_per_card_mib * len(devices)
    return usable, (
        f"{len(devices)} devices, {sum(devices)} MiB total, "
        f"{reserve_per_card_mib} MiB reserved on each"
    )


def per_card_ceiling(devices: list[int], reserve_per_card_mib: int = 512) -> int:
    """A model pinned to one card cannot exceed the smallest card."""
    return min(devices) - reserve_per_card_mib
```

Then in the existing NVIDIA branch replace the first-line read:

```python
        # BEFORE: return int(out.strip().splitlines()[0]), "nvidia vram"
        devices = parse_device_totals(out)
        usable, why = device_budget(devices)
        return usable, f"nvidia vram ({why})"
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-qwen38/tests/test_select_quant_devices.py -v`
Expected: 5 passed

- [ ] **Step 5: Confirm the 4090 node's existing behaviour is unchanged**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-qwen38/tests/ -q`
Expected: no new failures versus `git stash`-ing the change. A single-device
host now reports `24564 - 512`; if an existing test pins the old raw total,
update that test and say so in the commit.

- [ ] **Step 6: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-qwen38/scripts/select-quant.py \
        llm/linux-qwen38/tests/test_select_quant_devices.py
git commit -m "fix(llm): size the budget from every GPU, not the first one

select-quant.py read nvidia-smi's first row, so on a two-card host it saw
11 GiB and concluded nothing fits. It now enumerates every device and charges
the per-card reserve once per card, because compute buffers and the CUDA
context are paid on each one rather than shared."
```

---

### Task 2: Multi-GPU VRAM guard for the launcher

`serve-profile.sh` refuses to start when free VRAM is below
`QWEN38_MIN_FREE_VRAM_MIB` (default 20000, tuned for a 24 GiB card). It reads
`nvidia-smi --query-gpu=memory.free | head -1` — card 0 only. On 2 × 11 GiB,
card 0 reports ~11000, so **the guard blocks a node that fits**.

**Files:**
- Create: `llm/linux-turing-dual/scripts/vram-guard.sh`
- Modify: `llm/linux-qwen38/scripts/serve-profile.sh` (the `free_mib` guard, ~line 47)
- Test: `llm/linux-turing-dual/tests/test_vram_guard.py`

**Interfaces:**
- Consumes: nothing.
- Produces: `vram-guard.sh --min-total <MiB> [--min-per-card <MiB>] [--devices <csv>]`,
  exit 0 = enough, exit 75 (`EX_TEMPFAIL`) = not enough, exit 69 = no nvidia-smi.
  Reads `NVIDIA_SMI` from the environment so tests can stub it. Task 12's unit
  calls it.

- [ ] **Step 1: Write the failing test**

```python
# llm/linux-turing-dual/tests/test_vram_guard.py
import os, pathlib, stat, subprocess, textwrap, pytest

GUARD = pathlib.Path(__file__).resolve().parents[1] / "scripts" / "vram-guard.sh"

def fake_smi(tmp_path, lines):
    """A stub nvidia-smi printing one free-MiB figure per line."""
    p = tmp_path / "nvidia-smi"
    p.write_text("#!/usr/bin/env bash\ncat <<'EOF'\n" + lines + "\nEOF\n")
    p.chmod(p.stat().st_mode | stat.S_IEXEC)
    return p

def run(tmp_path, lines, *args):
    env = dict(os.environ, NVIDIA_SMI=str(fake_smi(tmp_path, lines)))
    return subprocess.run(["bash", str(GUARD), *args],
                          capture_output=True, text=True, env=env)

def test_two_cards_sum_to_enough(tmp_path):
    # 2 x 11000 = 22000 total: passes a 20000 floor that card 0 alone fails
    r = run(tmp_path, "11000\n11000", "--min-total", "20000")
    assert r.returncode == 0, r.stderr

def test_single_card_below_floor_is_refused(tmp_path):
    r = run(tmp_path, "9000", "--min-total", "20000")
    assert r.returncode == 75
    assert "9000" in r.stderr

def test_per_card_floor_enforced_even_when_total_passes(tmp_path):
    # 20000 total is fine, but a model pinned to one card needs 8000 on ONE card
    r = run(tmp_path, "19000\n1000", "--min-total", "20000", "--min-per-card", "8000")
    assert r.returncode == 75
    assert "per-card" in r.stderr

def test_missing_nvidia_smi_is_a_distinct_failure(tmp_path):
    env = dict(os.environ, NVIDIA_SMI=str(tmp_path / "does-not-exist"))
    r = subprocess.run(["bash", str(GUARD), "--min-total", "1"],
                       capture_output=True, text=True, env=env)
    assert r.returncode == 69
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-turing-dual/tests/test_vram_guard.py -v`
Expected: FAIL — the script does not exist yet.

- [ ] **Step 3: Write minimal implementation**

```bash
#!/usr/bin/env bash
# Multi-GPU free-VRAM guard.
#
#   vram-guard.sh --min-total <MiB> [--min-per-card <MiB>]
#
# Exists because the single-card version read nvidia-smi's FIRST ROW and
# compared it to a floor sized for one big card. On two 11 GiB cards that
# refuses to start a node which fits comfortably.
#
# exit 0  enough VRAM      exit 75 not enough      exit 69 no nvidia-smi
set -euo pipefail

NVIDIA_SMI="${NVIDIA_SMI:-nvidia-smi}"
MIN_TOTAL=0
MIN_PER_CARD=0

while [ $# -gt 0 ]; do
  case "$1" in
    --min-total)    MIN_TOTAL="${2:?}";    shift 2 ;;
    --min-per-card) MIN_PER_CARD="${2:?}"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 64 ;;
  esac
done

command -v "$NVIDIA_SMI" >/dev/null 2>&1 || {
  echo "vram-guard: ${NVIDIA_SMI} not found" >&2; exit 69; }

raw="$("$NVIDIA_SMI" --query-gpu=memory.free --format=csv,noheader,nounits 2>/dev/null)" || {
  echo "vram-guard: ${NVIDIA_SMI} failed" >&2; exit 69; }

total=0 best=0 n=0
while read -r mib; do
  [ -n "$mib" ] || continue
  mib="${mib//[[:space:]]/}"
  total=$(( total + mib ))
  [ "$mib" -gt "$best" ] && best="$mib"
  n=$(( n + 1 ))
done <<< "$raw"

[ "$n" -gt 0 ] || { echo "vram-guard: no devices reported" >&2; exit 69; }

if [ "$total" -lt "$MIN_TOTAL" ]; then
  echo "vram-guard: refusing to start: ${total} MiB free across ${n} device(s), need ${MIN_TOTAL}" >&2
  exit 75
fi

if [ "$MIN_PER_CARD" -gt 0 ] && [ "$best" -lt "$MIN_PER_CARD" ]; then
  echo "vram-guard: refusing to start: best per-card free is ${best} MiB, need ${MIN_PER_CARD} on one card" >&2
  exit 75
fi

echo "vram-guard: ok -- ${total} MiB free across ${n} device(s), best card ${best} MiB"
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-turing-dual/tests/test_vram_guard.py -v`
Expected: 4 passed

- [ ] **Step 5: Point the shared launcher at the guard**

In `llm/linux-qwen38/scripts/serve-profile.sh`, replace the `free_mib` block
with a call that keeps the old behaviour on a single card and gains the
aggregate on several. Keep the existing variable name so profiles do not change:

```bash
  # Multi-GPU aware. The previous version read nvidia-smi's first row only,
  # which refuses to start a two-card node that fits. QWEN38_MIN_FREE_VRAM_MIB
  # is now a TOTAL; QWEN38_MIN_FREE_PER_CARD_MIB is optional and matters only
  # for models pinned to one card.
  guard="${QWEN38_VRAM_GUARD:-}"
  if [ -n "$guard" ] && [ -x "$guard" ]; then
    "$guard" --min-total "${QWEN38_MIN_FREE_VRAM_MIB}" \
             ${QWEN38_MIN_FREE_PER_CARD_MIB:+--min-per-card "${QWEN38_MIN_FREE_PER_CARD_MIB}"} \
      || exit $?
  fi
```

- [ ] **Step 6: Verify the 4090 node's launcher still parses**

Run: `cd /tmp/wt-turing && bash -n llm/linux-qwen38/scripts/serve-profile.sh && shellcheck -S error llm/linux-qwen38/scripts/serve-profile.sh`
Expected: no output from either (shellcheck may be absent; `bash -n` is the required one)

- [ ] **Step 7: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/scripts/vram-guard.sh \
        llm/linux-turing-dual/tests/test_vram_guard.py \
        llm/linux-qwen38/scripts/serve-profile.sh
git commit -m "fix(llm): guard total VRAM across cards, not the first card's

The launcher compared nvidia-smi's first row against a 20000 MiB floor sized
for a single 24 GiB card. On two 11 GiB cards, card 0 reports ~11000 and the
guard refused to start a node that fits. The floor is now a total, with an
optional per-card floor for models pinned to one device."
```

---

### Task 3: Site config loader and the leak guard

The repo is public. This task creates the mechanism that keeps the real host
out of git, and the test that fails the build if it ever gets in.

**Files:**
- Create: `llm/linux-turing-dual/scripts/site.sh`
- Create: `llm/linux-turing-dual/config/site.conf.example`
- Create: `llm/linux-turing-dual/tests/test_site.py`
- Create: `llm/linux-turing-dual/tests/test_structural.sh`

**Interfaces:**
- Consumes: nothing.
- Produces: sourcing `site.sh` exports `QT_NODE_ADDR`, `QT_UPLINK_IF`,
  `QT_MODELS_DIR`, `QT_PORT`, `QT_DASH_PORT`. `require_site()` exits **78**
  (`EX_CONFIG`) naming the file when a value is missing or still a placeholder.
  Every later script sources this.

- [ ] **Step 1: Write the failing test**

```python
# llm/linux-turing-dual/tests/test_site.py
import os, pathlib, subprocess, textwrap

HERE = pathlib.Path(__file__).resolve().parents[1]
SITE = HERE / "scripts" / "site.sh"

def run(tmp_path, body):
    cfg = tmp_path / "site.conf"
    cfg.write_text(body)
    return subprocess.run(
        ["bash", "-c", f'QT_SITE_CONF="{cfg}"; . "{SITE}"; require_site; '
                       'echo "$QT_NODE_ADDR|$QT_UPLINK_IF|$QT_PORT"'],
        capture_output=True, text=True)

def test_loads_a_complete_config(tmp_path):
    r = run(tmp_path, textwrap.dedent('''
        : "${QT_NODE_ADDR:=10.0.0.5}"
        : "${QT_UPLINK_IF:=eth1}"
        : "${QT_MODELS_DIR:=/srv/models}"
        : "${QT_PORT:=8080}"
        : "${QT_DASH_PORT:=8081}"
    '''))
    assert r.returncode == 0, r.stderr
    assert r.stdout.strip() == "10.0.0.5|eth1|8080"

def test_placeholder_is_rejected_with_ex_config(tmp_path):
    r = run(tmp_path, ': "${QT_NODE_ADDR:=<node-addr>}"\n')
    assert r.returncode == 78
    assert "site.conf" in r.stderr

def test_missing_value_is_rejected_and_named(tmp_path):
    r = run(tmp_path, ': "${QT_NODE_ADDR:=10.0.0.5}"\n')
    assert r.returncode == 78
    assert "QT_UPLINK_IF" in r.stderr

def test_environment_wins_over_the_file(tmp_path):
    cfg = tmp_path / "site.conf"
    cfg.write_text(': "${QT_PORT:=8080}"\n')
    r = subprocess.run(
        ["bash", "-c", f'QT_SITE_CONF="{cfg}"; QT_PORT=9999; . "{SITE}"; echo "$QT_PORT"'],
        capture_output=True, text=True)
    assert r.stdout.strip() == "9999"

def test_missing_file_is_ex_config_not_a_crash(tmp_path):
    r = subprocess.run(
        ["bash", "-c", f'QT_SITE_CONF="{tmp_path}/nope.conf"; . "{SITE}"; require_site'],
        capture_output=True, text=True)
    assert r.returncode == 78
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-turing-dual/tests/test_site.py -v`
Expected: FAIL — `site.sh` does not exist.

- [ ] **Step 3: Write minimal implementation**

```bash
#!/usr/bin/env bash
# Load the site's own coordinates. This repository is public, so the real
# address, interface and model path are NOT committed -- they live in
# site.conf, and the committed example carries <angle-bracket> placeholders.
#
# The `: "${VAR:=value}"` form in site.conf means an already-exported
# environment variable wins, so a one-off override needs no file edit.
#
# require_site() exits 78 (EX_CONFIG) naming the file. That is not a bug.

QT_SITE_CONF="${QT_SITE_CONF:-${XDG_CONFIG_HOME:-$HOME/.config}/qwen-turing/site.conf}"

if [ -r "$QT_SITE_CONF" ]; then
  # shellcheck disable=SC1090
  . "$QT_SITE_CONF"
fi

require_site() {
  local missing="" v
  for v in QT_NODE_ADDR QT_UPLINK_IF QT_MODELS_DIR QT_PORT QT_DASH_PORT; do
    local val="${!v:-}"
    if [ -z "$val" ]; then
      missing="${missing} ${v}"
    elif case "$val" in *"<"*">"*) true ;; *) false ;; esac; then
      missing="${missing} ${v}(placeholder)"
    fi
  done
  if [ -n "$missing" ]; then
    echo "site.conf is incomplete: ${missing}" >&2
    echo "  edit ${QT_SITE_CONF} (copy from config/site.conf.example)" >&2
    return 78
  fi
  export QT_NODE_ADDR QT_UPLINK_IF QT_MODELS_DIR QT_PORT QT_DASH_PORT
}
```

Note `return 78` rather than `exit 78`: the file is sourced, and a RETURN trap
or `exit` inside a sourced file kills the caller's shell. Callers use
`require_site || exit $?`.

- [ ] **Step 4: Write the example config**

```bash
# llm/linux-turing-dual/config/site.conf.example
# Copy to ${XDG_CONFIG_HOME:-~/.config}/qwen-turing/site.conf and fill in.
# chmod 600. These are properties of a site, not of this tooling, and this
# repository is public.
#
# The `: "${VAR:=value}"` form means an environment variable already set wins
# over this file.

# The address the inference endpoint binds. Nothing else may bind it.
: "${QT_NODE_ADDR:=<node-addr>}"

# The interface that address lives on.
: "${QT_UPLINK_IF:=<uplink-if>}"

# Where GGUF weights live. Must be fast and roomy -- NOT the root filesystem.
: "${QT_MODELS_DIR:=<nvme-array>/qwen-gguf}"

# Inference port, and the stats dashboard's port.
: "${QT_PORT:=8080}"
: "${QT_DASH_PORT:=8081}"
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-turing-dual/tests/test_site.py -v`
Expected: 5 passed

- [ ] **Step 6: Write the structural test, including the leak guard**

```bash
#!/usr/bin/env bash
# No-accelerator checks for the dual-Turing node. Runs in CI.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NODE="$(dirname "$HERE")"
root="$(git -C "$NODE" rev-parse --show-toplevel)"
fails=0
ok()  { echo "ok   -- $*"; }
bad() { echo "FAIL -- $*" >&2; fails=$(( fails + 1 )); }

# 1 -- every script the installer ships must exist and parse.
for s in site.sh vram-guard.sh install-node.sh; do
  f="${NODE}/scripts/${s}"
  [ -r "$f" ] || { bad "missing scripts/${s}"; continue; }
  bash -n "$f" 2>/dev/null && ok "scripts/${s} parses" || bad "scripts/${s} has a syntax error"
done

# 2 -- the example must cover every value require_site() demands, or a fresh
# operator hits exit 78 with no way to know what to write.
ex="${NODE}/config/site.conf.example"
demanded="$(grep -oE 'QT_[A-Z_]+' "${NODE}/scripts/site.sh" | sort -u \
            | grep -vE 'QT_SITE_CONF')"
missing=""
for v in $demanded; do
  grep -q "$v" "$ex" || missing="${missing} ${v}"
done
[ -z "$missing" ] && ok "site.conf.example covers every demanded value" \
                  || bad "site.conf.example does not set:${missing}"

# 3 -- LEAK GUARD. This repository is public. No real site identifier may be
# committed. Patterns are read from an env var so this file does not itself
# contain the thing it forbids.
#   QT_LEAK_PATTERNS='addr1|host1|domain' tests/test_structural.sh
leak_pat="${QT_LEAK_PATTERNS:-}"
if [ -n "$leak_pat" ]; then
  hits="$(git -C "$root" grep -niIE "$leak_pat" -- llm docs/memory 2>/dev/null \
          | grep -v 'leak-guard-allow' || true)"
  if [ -n "$hits" ]; then
    bad "a real site identifier is committed:"
    echo "$hits" | sed 's/^/        /' | head -5 >&2
  else
    ok "no real site identifier is committed"
  fi
else
  ok "leak guard skipped (QT_LEAK_PATTERNS unset)"
fi

# 4 -- placeholders must remain placeholders in committed config.
for f in "${NODE}"/config/*.conf "${NODE}"/config/*.example "${NODE}"/config/*.ini; do
  [ -r "$f" ] || continue
  if grep -qE '^\s*:?\s*"?\$\{QT_NODE_ADDR:=[0-9]{1,3}\.' "$f"; then
    bad "$(basename "$f") hardcodes a literal address"
  fi
done
ok "committed config carries no literal address"

# 5 -- the CUDA sandboxing ceiling must not be tightened back.
u="${NODE}/systemd/qwen-turing@.service"
if [ -r "$u" ]; then
  for forbidden in 'ProtectSystem=strict' 'PrivateDevices=true' \
                   'DevicePolicy=closed' 'MemoryDenyWriteExecute=yes'; do
    grep -qE "^\s*${forbidden}" "$u" \
      && bad "unit sets ${forbidden}, which breaks CUDA on this host"
  done
  grep -qE '^\s*ProtectSystem=full' "$u" \
    && ok "unit uses ProtectSystem=full" \
    || bad "unit must set ProtectSystem=full"
fi

echo
[ "$fails" -eq 0 ] && { echo "OK -- all structural checks passed"; exit 0; }
echo "${fails} structural check(s) failed" >&2; exit 1
```

- [ ] **Step 7: Run the structural test**

Run: `cd /tmp/wt-turing && chmod +x llm/linux-turing-dual/tests/test_structural.sh llm/linux-turing-dual/scripts/*.sh && bash llm/linux-turing-dual/tests/test_structural.sh`
Expected: `OK -- all structural checks passed`. Check 5 is skipped until Task 12
writes the unit; that is correct at this point.

- [ ] **Step 8: Prove the leak guard actually catches a leak**

A guard that has never fired is a guard you do not know works.

Run:
```bash
cd /tmp/wt-turing
echo "# probe zzunique-marker-token" > llm/linux-turing-dual/LEAKTEST.md
git add llm/linux-turing-dual/LEAKTEST.md
QT_LEAK_PATTERNS='zzunique-marker-token' bash llm/linux-turing-dual/tests/test_structural.sh; echo "exit=$?"
```
Expected: `FAIL -- a real site identifier is committed` and `exit=1`.
Then undo: `git rm -q --cached llm/linux-turing-dual/LEAKTEST.md && rm llm/linux-turing-dual/LEAKTEST.md`
A non-IP token is used deliberately: a sample address in the plan would itself
match a future leak-guard run and read as a leak.

- [ ] **Step 9: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/scripts/site.sh \
        llm/linux-turing-dual/config/site.conf.example \
        llm/linux-turing-dual/tests/test_site.py \
        llm/linux-turing-dual/tests/test_structural.sh
git commit -m "feat(llm): keep the dual-Turing node's site coordinates out of git

The repository is public, so the address, interface and model path load from a
local site.conf and the committed example carries placeholders. require_site
returns 78 naming the file rather than failing somewhere later with an empty
bind address.

The leak guard reads its patterns from the environment so this test does not
itself contain the identifiers it forbids, and step 8 of the plan fires it
deliberately -- an untested guard is not a guard."
```

---

### Task 4: CI wiring — because none of this currently runs

`llm/` appears in none of the twelve workflows, so the leak guard has never
gated a commit. The structural and unit tests need no accelerator.

**Files:**
- Modify: `.github/workflows/python.yml`
- Test: the workflow itself, verified by `act` or by reading the run

**Interfaces:**
- Consumes: `tests/test_structural.sh`, `tests/test_site.py`, `tests/test_vram_guard.py` from Tasks 1-3.
- Produces: a CI job named `llm-node-structural`.

- [ ] **Step 1: Read the existing workflow to match its conventions**

Run: `cd /tmp/wt-turing && cat .github/workflows/python.yml`
Note the runner, the Python setup action and version, and whether a matrix is
used. Match them; do not invent a second style.

- [ ] **Step 2: Add the job**

Append a job to `.github/workflows/python.yml` mirroring the file's existing
conventions. It must run:

```yaml
      - name: Dual-Turing node structural checks
        run: bash llm/linux-turing-dual/tests/test_structural.sh

      - name: Node unit tests (no accelerator)
        run: |
          python3 -m pytest \
            llm/linux-turing-dual/tests/test_site.py \
            llm/linux-turing-dual/tests/test_vram_guard.py \
            llm/linux-qwen38/tests/test_select_quant_devices.py -q
```

Do **not** run `llm/linux-qwen38/tests/test_smoke.sh` or `test_vision.py` in
CI — they need a live server and a GPU.

- [ ] **Step 3: Verify the workflow is valid YAML and the paths exist**

Run:
```bash
cd /tmp/wt-turing
python3 -c "import yaml,sys; d=yaml.safe_load(open('.github/workflows/python.yml')); print(sorted(d['jobs']))"
for f in llm/linux-turing-dual/tests/test_structural.sh \
         llm/linux-turing-dual/tests/test_site.py \
         llm/linux-turing-dual/tests/test_vram_guard.py \
         llm/linux-qwen38/tests/test_select_quant_devices.py; do
  test -r "$f" && echo "ok $f" || echo "MISSING $f"
done
```
Expected: the job list includes the new job; every path prints `ok`.

- [ ] **Step 4: Run exactly what CI will run**

Run:
```bash
cd /tmp/wt-turing
bash llm/linux-turing-dual/tests/test_structural.sh && \
python3 -m pytest llm/linux-turing-dual/tests/test_site.py \
                  llm/linux-turing-dual/tests/test_vram_guard.py \
                  llm/linux-qwen38/tests/test_select_quant_devices.py -q
```
Expected: structural OK, and all unit tests pass.

- [ ] **Step 5: Commit**

```bash
cd /tmp/wt-turing
git add .github/workflows/python.yml
git commit -m "ci(llm): actually run the inference-node checks

llm/ appeared in none of the twelve workflows, so the public-repo leak guard
and the preset arithmetic only ran when somebody remembered. The structural and
unit checks need no accelerator, so they run on every push; the smoke and
vision suites still do not, because they need a live server on a real GPU."
```

---

### Task 5: Tier A host cleanup — reversible by reinstall

Host operations, not code. "Test" here means a verification command with an
expected result. Every removal in this task is reversible by reinstalling a
package.

**Files:**
- Create: `llm/linux-turing-dual/ops/cleanup-tier-a.sh`
- Create: `llm/linux-turing-dual/docs/measured-ceilings.md` (started here, filled in Task 13)

**Interfaces:**
- Consumes: nothing.
- Produces: `cleanup-tier-a.sh [--dry-run]`. Default is `--dry-run`; `--apply`
  is required to change anything. Task 6 runs after it.

- [ ] **Step 1: Record the before state, so the win is measurable**

Run on the node:
```bash
df -h / | tail -1 | tee /tmp/qt-before-df.txt
dpkg -l | grep -c '^ii'
snap list | wc -l
```
Expected: root ~84% used. **Write these numbers into
`docs/measured-ceilings.md` under "Host cleanup — before".** A cleanup with no
before-figure cannot be shown to have worked.

- [ ] **Step 2: Write the script, dry-run by default**

`ops/cleanup-tier-a.sh` must:
1. Refuse to run unless `--apply` is passed; print what it would do otherwise.
2. Remove the desktop stack:
   `ubuntu-desktop ubuntu-desktop-minimal gnome-shell gnome-shell-common gdm3 xserver-xorg-core libreoffice-style-* thunderbird cups-daemon davinci-resolve jenkins`
3. **Install `nvidia-headless-580` and `nvidia-utils-580` before removing
   `nvidia-driver-580`**, in that order, in one `apt-get install` invocation so
   apt resolves it as a swap rather than a removal followed by a gap.
4. Remove driver sediment:
   `libnvidia-compute-390 libnvidia-compute-418 libnvidia-compute-430 libnvidia-compute-470 libnvidia-compute-525 libnvidia-compute-535 nvidia-dkms-435 nvidia-dkms-525 nvidia-dkms-535 nvidia-driver-525 nvidia-driver-535 nvidia-utils-535 cuda-toolkit-10-0`
5. Remove kernels **5.3.0-26, 5.15.0-153, 6.8.0-84 only**. It must
   `exit 1` if asked to touch 6.8.0-101, 111 or 138 — 101 is the fallback held
   by Barrier 2, 111 is running, 138 is the next boot target.
6. Remove desktop snaps: `chromium firefox cups dbeaver-ce gthumb` and the
   `gnome-3-26-1604 gnome-3-28-1804 gnome-3-34-1804 gnome-3-38-2004 gnome-42-2204` platform snaps.
7. `rm -rf` the `/opt` relics: `resolve idea-IU-181.4203.550 idea-IC-173.4548.28 pycharm-2017.3.4 ideaIU-2018.1.tar.gz WebStorm-181.4203.535 monero-gui-v0.11.1.0`
8. `journalctl --vacuum-size=200M`, `apt-get clean`, `apt-get autoremove --purge`,
   and remove the obsolete `mlocate` (`plocate` is already installed).
9. Print a `df -h /` before and after.

- [ ] **Step 3: Dry-run it and read every line**

Run: `bash llm/linux-turing-dual/ops/cleanup-tier-a.sh --dry-run`
Expected: a list of removals. **Confirm no `linux-image-6.8.0-101`, `-111` or
`-138` appears.** If one does, fix the script — do not proceed.

- [ ] **Step 4: Verify apt agrees it is a swap, not a decapitation**

The single most dangerous line is the nvidia swap. Ask apt first:
```bash
sudo apt-get install --dry-run nvidia-headless-580 nvidia-utils-580 2>&1 | grep -E "^(Inst|Remv|REMOVING)" | head -30
```
Expected: `Inst nvidia-headless-580`, and **no `Remv` of any
`libnvidia-compute-580`, `nvidia-dkms-580` or `nvidia-kernel-common-580`**. If
apt proposes removing the 580 DKMS package, stop: that is the path to a
GPU-less host, and Barrier 1 exists to catch it.

- [ ] **Step 5: Apply**

Run: `sudo bash llm/linux-turing-dual/ops/cleanup-tier-a.sh --apply 2>&1 | tee /tmp/qt-tier-a.log`
Expected: completes without `E:` lines from apt.

- [ ] **Step 6: Verify — this is Barrier 1's first half**

```bash
sudo dkms status
ls /lib/modules/*/updates/dkms/nvidia.ko*
dpkg -l | grep -E '^ii.*nvidia-(headless|utils|dkms)-580'
df -h /
```
Expected: `nvidia/580.173.02` installed for **both** `6.8.0-111-generic` and
`6.8.0-138-generic`; `nvidia.ko.zst` present under both; root usage materially
down. **If DKMS lost either kernel, do not reboot** — rebuild with
`sudo dkms autoinstall` and re-verify.

- [ ] **Step 7: Commit the script and the recorded numbers**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/ops/cleanup-tier-a.sh \
        llm/linux-turing-dual/docs/measured-ceilings.md
git commit -m "feat(llm): reversible host cleanup for the dual-Turing node

Dry-run by default, because the destructive form of this script is one typo
from removing the running kernel. It refuses outright to touch 6.8.0-101, -111
or -138: 111 is running, 138 is the next boot target and has never booted here,
and 101 is the fallback held until it does.

nvidia-headless-580 is installed in the same apt invocation that removes
nvidia-driver-580 so apt resolves a swap rather than leaving a gap where the
DKMS module used to be."
```

---

### Task 6: Tier B and Tier C — moves, then authorised deletions

Tier B moves rather than deletes. Tier C is the operator-authorised destructive
work and carries Barrier 4.

**Files:**
- Create: `llm/linux-turing-dual/ops/cleanup-tier-bc.sh`

**Interfaces:**
- Consumes: `cleanup-tier-a.sh` has already run.
- Produces: nothing later tasks read.

- [ ] **Step 1: Inspect `~/.lmstudio` before moving it**

It may already hold GGUF weights, which would save a download in Task 11.

Run: `find ~/.lmstudio -name '*.gguf' -printf '%s\t%p\n' 2>/dev/null | sort -rn | head -20`
Record any Qwen GGUF found, with its exact size, in `docs/measured-ceilings.md`.

- [ ] **Step 2: Write the Tier B moves**

In `ops/cleanup-tier-bc.sh`, `--apply`-gated as before:
1. `mkdir -p <bulk-array>/archive/libvirt-images` and **`mv`** (not `cp`, not
   `rm`) `/var/lib/libvirt/images/*.qcow2` there. Leave the domains defined.
2. **`mv`** `~/.lmstudio` to `<nvme-array>/lmstudio` and leave a symlink behind,
   so anything still pointing at it keeps working.
3. `rm -rf` the stale JetBrains caches: `~/.PyCharm2018.1 ~/.PyCharm2019.1 ~/.PyCharm2019.3 ~/.IntelliJIdea2018.1`

- [ ] **Step 3: Verify the moves preserved the data**

```bash
ls -lh <bulk-array>/archive/libvirt-images/
sudo virsh list --all | head
df -h / <bulk-array> <nvme-array>
```
Expected: both qcow2 files present at their original sizes; all eight domains
still listed; root usage down by ~36 GiB.

- [ ] **Step 4: Barrier 4 — verify array health inside the delete step**

This must run **immediately before** the account removal, not earlier:

```bash
cat /proc/mdstat
sudo test -r <bulk-array>/backup/leon && sudo du -sh <bulk-array>/backup/leon
```
Expected: no `[U_]` or `_U` degraded markers in `mdstat`, and the backup reports
~18 GiB. **If either check fails, skip the account removal entirely** — the
backup is `/home/leon`'s only other copy.

- [ ] **Step 5: Remove the four accounts**

Verified already: no running processes, no crontabs, no files outside `$HOME`,
never logged in. Preserve `<bulk-array>/backup/leon`.

```bash
for u in jvogel diego sajjan; do sudo userdel "$u" 2>&1; done   # no -r: no home dirs worth keeping
sudo gpasswd -d leon docker 2>/dev/null || true
sudo userdel -r leon
getent passwd leon jvogel diego sajjan || echo "all four removed"
sudo du -sh <bulk-array>/backup/leon
```
Expected: `all four removed`, and the backup still reports ~18 GiB.
Note `jvogel` has a 92 KiB home; use `-r` for it too if you want it gone.

- [ ] **Step 6: Prune Docker — after the accounts, so an array fault surfaces cheaply first**

```bash
docker service rm denmark_binbase denmark_binview denmark_node
docker rm wcmc_postgresql13_1 binbase.steinlee
docker image prune -a -f
docker system df
```
Expected: `Images` drops to near zero reclaimable. **Do not run
`docker volume prune`** — 64 volumes hold data and one is Postgres-shaped;
volumes are explicitly out of scope.

- [ ] **Step 7: Record the after state**

Run: `df -h / <bulk-array> <nvme-array> | tee -a /tmp/qt-after-df.txt`
Expected: root **below 50%** (acceptance criterion 8). Write before/after into
`docs/measured-ceilings.md`.

- [ ] **Step 8: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/ops/cleanup-tier-bc.sh \
        llm/linux-turing-dual/docs/measured-ceilings.md
git commit -m "feat(llm): move what can be moved, delete only what was authorised

Tier B moves rather than deletes -- the 2019 qcow2 images go to the bulk array
with their domains left defined, so nothing is destroyed to reclaim root.

Tier C carries the barrier that matters: /home/leon's only other copy is on the
bulk array, so mdstat and the backup's readability are checked inside the delete
step rather than inherited from an earlier phase, and failing either leaves the
account alone. Docker volumes are untouched: 64 of them hold data and one is a
Postgres volume."
```

---

### Task 7: Headless switch, then THE REBOOT BARRIER

**Files:**
- Create: `llm/linux-turing-dual/ops/prep-headless.sh`

**Interfaces:**
- Consumes: Tasks 5-6 complete.
- Produces: a host with coherent drivers and both GPUs visible. **Every
  subsequent task depends on this.**

- [ ] **Step 1: Set the boot target and confirm which kernel will boot**

```bash
sudo systemctl set-default multi-user.target
sudo systemctl disable gdm3 2>/dev/null || true
systemctl get-default
grep -E '^GRUB_DEFAULT' /etc/default/grub
ls -1 /boot/vmlinuz-* | sort -V | tail -1
```
Expected: `multi-user.target`; `GRUB_DEFAULT=0`; newest kernel is
`6.8.0-138-generic`, which is therefore the boot target.

- [ ] **Step 2: Barrier 1, second half — re-verify DKMS for the ACTUAL boot target**

```bash
sudo dkms status | grep 6.8.0-138
ls -l /lib/modules/6.8.0-138-generic/updates/dkms/nvidia.ko.zst
```
Expected: `nvidia/580.173.02, 6.8.0-138-generic, x86_64: installed` and the
module file present.

**STOP if either is missing.** Run `sudo dkms autoinstall`, re-verify, and only
then continue. A reboot without this module returns a host with no GPU and no
way to run anything else in this plan.

- [ ] **Step 3: Get explicit operator confirmation for the reboot**

The operator has a graphical session on `tty2` and live SSH sessions. **Do not
reboot on standing authorization.** Ask, and wait.

- [ ] **Step 4: Reboot**

```bash
sudo systemctl reboot
```
Then wait for the host and reconnect.

- [ ] **Step 5: Verify the barrier was crossed successfully**

```bash
uname -r
nvidia-smi
nvidia-smi --query-gpu=index,name,memory.total,memory.free,compute_cap,driver_version --format=csv
nvidia-smi topo -m
nvidia-smi nvlink -s 2>&1 | head
systemctl get-default
pgrep -a Xorg || echo "no Xorg -- both cards free"
```
Expected: kernel `6.8.0-138-generic`; **`nvidia-smi` succeeds** (the original
NVML mismatch is gone); two `NVIDIA GeForce RTX 2080 Ti` rows at 11264 MiB with
`compute_cap 7.5`; **free VRAM within ~200 MiB of total on both cards**; no
Xorg. Record the topology and whether NVLink is fitted in
`docs/measured-ceilings.md`.

- [ ] **Step 6: Now that 138 has booted cleanly, retire the fallback (Barrier 2 released)**

```bash
sudo apt-get purge -y linux-image-6.8.0-101-generic linux-modules-6.8.0-101-generic \
                      linux-modules-extra-6.8.0-101-generic linux-headers-6.8.0-101 \
                      linux-headers-6.8.0-101-generic
df -h /boot /
```
Expected: `/boot` usage down; `6.8.0-111` and `6.8.0-138` still present.

- [ ] **Step 7: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/ops/prep-headless.sh \
        llm/linux-turing-dual/docs/measured-ceilings.md
git commit -m "feat(llm): make the node headless and record what the reboot proved

Going headless is a package fact, not a disabled unit: nvidia-headless-580
replaces the -driver- metapackage that pulls in X, so Xorg cannot come back and
reclaim VRAM on both cards.

The DKMS module is re-verified against the actual boot target immediately before
the reboot, because the earlier check predates the package purge. Kernel
6.8.0-101 is retired only after 6.8.0-138 has booted with working GPUs -- the
fallback outlives the risk it covers."
```

---

## Phase B — post-reboot. Nothing here may be attempted before Task 7 Step 5 passes.

### Task 8: Record the cards in the GPU registry

**Files:**
- Modify: `llm/linux-qwen38/config/gpu-registry.json`

**Interfaces:**
- Consumes: `nvidia-smi` output from Task 7 Step 5.
- Produces: a registry entry keyed exactly as `nvidia-smi` names the card, read
  by `select-quant.py` and the roofline prediction.

- [ ] **Step 1: Read the observed values rather than typing remembered ones**

Run: `nvidia-smi --query-gpu=name,memory.total,compute_cap --format=csv,noheader`
Expected: two identical rows. Use the **exact** name string as the JSON key.

- [ ] **Step 2: Add the entry**

`vram_mib` and `compute_cap` are OBSERVED. `bandwidth_gbs` is the vendor figure
and is the one number here that is not measured — the existing file's schema
keeps that distinction, so honour it. `measured_tps` starts empty and is filled
by Task 13, not now.

```json
    "NVIDIA GeForce RTX 2080 Ti": {
      "vram_mib": 11264,
      "compute_cap": "7.5",
      "bandwidth_gbs": 616,
      "fp8": false,
      "nvfp4": false,
      "measured_tps": {},
      "seen_at": [
        "dual-turing-node"
      ]
    },
```

- [ ] **Step 3: Verify the file is still valid JSON and the entry reads back**

Run:
```bash
cd /tmp/wt-turing
python3 -c "
import json
d=json.load(open('llm/linux-qwen38/config/gpu-registry.json'))
g=d['gpus']['NVIDIA GeForce RTX 2080 Ti']
assert g['compute_cap']=='7.5' and g['fp8'] is False and g['nvfp4'] is False
assert g['measured_tps']=={}, 'measured_tps must stay empty until measured'
print('ok', g['vram_mib'], 'MiB x2 =', g['vram_mib']*2)
"
```
Expected: `ok 11264 MiB x2 = 22528`

- [ ] **Step 4: Check the roofline prediction the registry now implies**

Run:
```bash
cd /tmp/wt-turing
python3 -c "
bw, eff = 616, 0.8
for name, gb in (('27B UD-Q4_K_M', 16.46), ('9B Q4_K_M', 5.68)):
    print(f'{name:16s} predicted {bw/gb*eff:5.1f} tok/s')
"
```
Expected: ~29.9 and ~86.8 tok/s. These are **predictions**; Task 13 replaces
them with measurements.

- [ ] **Step 5: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-qwen38/config/gpu-registry.json
git commit -m "feat(llm): record the RTX 2080 Ti as observed, not as remembered

vram_mib and compute_cap come from nvidia-smi on the card itself; fp8 and nvfp4
are false because Turing has neither, which is what rules out the quantisation
formats the 4090 node can use. bandwidth_gbs stays flagged as the vendor figure,
and measured_tps is deliberately empty until a benchmark fills it."
```

---

### Task 9: Build llama.cpp for sm_75

**Files:**
- Create: `llm/linux-turing-dual/config/common.conf`
- Create: `llm/linux-turing-dual/scripts/install-node.sh` (build phase only; later steps added in Tasks 10-12)

**Interfaces:**
- Consumes: `site.sh` from Task 3.
- Produces: `${QT_PREFIX}/llama.cpp/current/llama-server`, and `common.conf`
  exporting `QT_PREFIX QT_STATE QT_LLAMA_DIR QT_LLAMA_TAG QT_MIN_FREE_VRAM_MIB QT_MIN_FREE_PER_CARD_MIB`.

- [ ] **Step 1: Pick and pin a release tag**

Run: `git -c versionsort.suffix=- ls-remote --tags --sort=-v:refname https://github.com/ggml-org/llama.cpp 'refs/tags/b*' | head -5`
Record the chosen tag in `common.conf` as `QT_LLAMA_TAG`. A moving `master` is
not a pin, and the existing node learned this: build a *pinned release*, and
make the pin mean something.

- [ ] **Step 2: Write `common.conf`**

```bash
# Dual-Turing Qwen node -- settings shared by everything.
# shellcheck shell=bash
QT_PREFIX="/opt/qwen-turing"
QT_STATE="/var/lib/qwen-turing"
QT_LLAMA_DIR="${QT_PREFIX}/llama.cpp/current"
QT_LLAMA_TAG="<pinned-tag-from-step-1>"

# sm_75. Turing has no bf16, no FP8, no NVFP4 and no Marlin. Building for the
# build node's native arch instead of this pin produced a binary that ran only
# on the machine that compiled it.
QT_CUDA_ARCHS="75"

# TOTAL free VRAM across the cards, not the first card's. The single-card
# version of this guard refused to start a two-card node that fits.
QT_MIN_FREE_VRAM_MIB="19000"
# Only matters for a model pinned to one card -- the 9B needs ~7.1 GiB of one.
QT_MIN_FREE_PER_CARD_MIB="8000"

QT_VRAM_GUARD="${QT_PREFIX}/bin/vram-guard.sh"
```

- [ ] **Step 3: Build**

```bash
sudo mkdir -p "${QT_PREFIX}" "${QT_STATE}"
src=/usr/local/src/llama.cpp
sudo git clone --depth 1 --branch "<pinned-tag>" https://github.com/ggml-org/llama.cpp "$src"
sudo cmake -S "$src" -B "$src/build" \
  -DGGML_CUDA=ON \
  -DCMAKE_CUDA_ARCHITECTURES=75 \
  -DGGML_NATIVE=OFF \
  -DCMAKE_BUILD_TYPE=Release
sudo cmake --build "$src/build" --config Release -j"$(nproc)"
```

`-DGGML_NATIVE=OFF` is deliberate: a native build targets the build node's CPU
and is not portable. `-DCMAKE_CUDA_ARCHITECTURES=75` is what makes it run on
Turing at all.

- [ ] **Step 4: Verify the binary reports the right architecture and sees both cards**

```bash
sudo install -d "${QT_LLAMA_DIR}"
sudo cp "$src"/build/bin/llama-* "${QT_LLAMA_DIR}/"
sudo cp "$src"/build/bin/*.so "${QT_LLAMA_DIR}/" 2>/dev/null || true
LD_LIBRARY_PATH="${QT_LLAMA_DIR}" "${QT_LLAMA_DIR}/llama-server" --version
LD_LIBRARY_PATH="${QT_LLAMA_DIR}" "${QT_LLAMA_DIR}/llama-server" --list-devices
```
Expected: the version matches the pinned tag; `--list-devices` lists **two**
CUDA devices. One device means the build or the driver is wrong — stop here.

- [ ] **Step 5: Install the guard next to the binaries**

```bash
sudo install -m 0755 llm/linux-turing-dual/scripts/vram-guard.sh "${QT_PREFIX}/bin/vram-guard.sh"
sudo "${QT_PREFIX}/bin/vram-guard.sh" --min-total 19000 --min-per-card 8000
```
Expected: `vram-guard: ok -- ~22400 MiB free across 2 device(s)`. This is the
guard that would have blocked startup before Task 2.

- [ ] **Step 6: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/config/common.conf \
        llm/linux-turing-dual/scripts/install-node.sh
git commit -m "feat(llm): build llama.cpp for Turing, from a pin, portably

CMAKE_CUDA_ARCHITECTURES=75 is what makes the binary run on these cards at all,
and GGML_NATIVE=OFF stops it being tied to the CPU that compiled it. The tag is
pinned rather than tracking master, because 'the latest build' is not a
configuration anyone can reproduce.

The VRAM floor is a total across both cards with a separate per-card floor for
the 9B, which is pinned to one device."
```

---

### Task 10: Fetch and pin the weights

**Files:**
- Create: `llm/linux-turing-dual/config/model-artifacts.yaml`

**Interfaces:**
- Consumes: `QT_MODELS_DIR` from `site.sh`.
- Produces: two GGUF files on `<nvme-array>`, and provenance recorded by
  immutable revision. Task 11's presets reference these exact paths.

- [ ] **Step 1: Reuse anything already on disk**

If Task 6 Step 1 found a usable Qwen GGUF in the moved `~/.lmstudio`, use it
rather than re-downloading, but **verify its identity** before trusting it —
a filename is not a provenance.

- [ ] **Step 2: Resolve both repos to immutable revisions**

```bash
for r in unsloth/Qwen3.8-27B-GGUF unsloth/Qwen3.5-9B-GGUF; do
  echo "$r: $(curl -sfL "https://huggingface.co/api/models/$r" | python3 -c 'import json,sys; print(json.load(sys.stdin)["sha"])')"
done
```
Record both SHAs. Changing weights later is a **new profile version**, never an
edit in place.

- [ ] **Step 3: Download to the NVMe array, never to `/`**

```bash
. llm/linux-turing-dual/scripts/site.sh && require_site
sudo install -d -o root -g root "${QT_MODELS_DIR}"
cd "${QT_MODELS_DIR}"
sudo curl -fL -o Qwen3.8-27B-UD-Q4_K_M.gguf \
  "https://huggingface.co/unsloth/Qwen3.8-27B-GGUF/resolve/<sha-27b>/Qwen3.8-27B-UD-Q4_K_M.gguf"
sudo curl -fL -o Qwen3.5-9B-Q4_K_M.gguf \
  "https://huggingface.co/unsloth/Qwen3.5-9B-GGUF/resolve/<sha-9b>/Qwen3.5-9B-Q4_K_M.gguf"
```

- [ ] **Step 4: Verify sizes match what was planned**

```bash
ls -l --block-size=M "${QT_MODELS_DIR}"
df -h "${QT_MODELS_DIR}" /
```
Expected: ~15700 M (16.46 GB) and ~5420 M (5.68 GB). A size materially different
from the plan means a different quantisation was fetched — the VRAM budget in
the spec no longer applies and must be redone. **`/` must be unchanged.**

- [ ] **Step 5: Write `model-artifacts.yaml`**

Mirror the existing node's schema. For each artifact record `id`, `role`,
`runtime`, `runtime_version` (the pinned llama.cpp tag), `repository`,
`revision`, `on_disk_gib`, `architecture: Qwen3_5ForConditionalGeneration`,
`quantization.variant`, and a `context` block with
`n_ctx_train: 262144` and **`verified_max: null`** — null until Task 13
measures it. Record the KV arithmetic per model as a comment:
27B = 16 of 64 full-attn → 18.0 KiB/token at q4_0;
9B = 8 of 32 full-attn → 9.0 KiB/token at q4_0.

- [ ] **Step 6: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/config/model-artifacts.yaml
git commit -m "feat(llm): pin both checkpoints by revision, not by nickname

A bit-width nickname is not an identity: Q4_K_M from two uploaders are different
files. Both artifacts are pinned to a repository revision, and verified_max is
null because nothing has retrieved anything at length yet.

The KV arithmetic travels with each artifact so the next person sizing a pool
does not re-derive it: only 16 of the 27B's 64 layers and 8 of the 9B's 32 grow
a cache, which is the whole reason 40k seats are affordable on 11 GiB cards."
```

---

### Task 11: Router presets and the preset gate

**Files:**
- Create: `llm/linux-turing-dual/config/router-presets.ini`
- Create: `llm/linux-turing-dual/config/profiles.d/router.conf`

**Interfaces:**
- Consumes: model paths from Task 10, `common.conf` from Task 9.
- Produces: served ids `qwen3.8-27b` and `qwen3.5-9b` plus their tier aliases,
  read by `llama-server --models` and by `check_presets.py`.

- [ ] **Step 1: Write the presets**

Keys are `llama-server` long options without leading dashes. `tensor-split` is
the new key this node needs and the 4090 node never had.

```ini
; Dual-Turing router presets. One endpoint, one resident model, two seats.
;
; MUST be started with --models-max 1: the 27B holds ~15.3 GiB of 22 GiB and a
; second resident model would OOM.
;
; KV is derived, not guessed. bytes/token = 2 x N_full_attn x kv_heads x head_dim
;   27B: 16 of 64 full-attn, kv 4, head_dim 256 -> 18.0 KiB/token at q4_0
;    9B:  8 of 32 full-attn, kv 4, head_dim 256 ->  9.0 KiB/token at q4_0
;
; A shared pool has NO admission control: over-subscribe it and every live
; session dies together, not just the greedy one. That is why the -40k aliases
; exist -- rationing is client-side.

version = 1

[*]
n-gpu-layers = 999
flash-attn = auto
jinja = true
no-context-shift = true
cache-type-k = q4_0
cache-type-v = q4_0
kv-unified = true
; Within-slot prefix reuse: safe, and what repeated calls sharing a preamble
; benefit from. Cross-slot selection stays off -- see router.conf.
cache-reuse = 256

; --- 27B: layer-split across BOTH cards -------------------------------------
; 15.33 GiB weights + 1.41 GiB KV (81,920 @ q4_0) + per-card buffers and CUDA
; context leaves ~2.9 GiB of 22.0 GiB. Verified at length by measure-ceiling.sh.
[qwen3.8-27b]
model = <QT_MODELS_DIR>/Qwen3.8-27B-UD-Q4_K_M.gguf
c = 81920
parallel = 2
split-mode = layer
tensor-split = 1,1
alias = qwen3.8-27b-80k,qwen3.8-27b-40k

; --- 9B: pinned to ONE card -------------------------------------------------
; 5.29 GiB weights + 0.70 GiB KV fits one 11.0 GiB card with ~3.9 GiB spare, so
; it skips split overhead entirely. tensor-split puts all layers on device 0.
[qwen3.5-9b]
model = <QT_MODELS_DIR>/Qwen3.5-9B-Q4_K_M.gguf
c = 81920
parallel = 2
split-mode = layer
tensor-split = 1,0
alias = qwen3.5-9b-80k,qwen3.5-9b-40k
```

- [ ] **Step 2: Write `profiles.d/router.conf`**

```bash
# Profile: router. The only profile this node runs.
# shellcheck shell=bash
QT_PROFILE="router"
QT_PROFILE_VERSION="turing-dual-gguf-q4km-router-v1"
QT_RUNTIME="llama.cpp"
QT_ROUTER_PRESETS="${QT_PREFIX}/etc/router-presets.ini"

# ONE. Two of these models will not co-reside in 22 GiB.
QT_ROUTER_MAX_LOADED="1"

# Cross-slot KV reuse stays OFF. This is the path disabled in commit 1cd8c1f4
# because the model child kept dying on it. It is also, unhelpfully, the half a
# two-session fan-out would benefit from most. Do not "improve" this.
QT_SLOT_PROMPT_SIMILARITY="0.0"
```

- [ ] **Step 3: Verify no preset over-commits its pool or its slots**

```bash
cd /tmp/wt-turing
python3 llm/linux-qwen38/tests/check_presets.py \
  --config llm/linux-turing-dual/config/router-presets.ini
```
Expected: pass. A tier advertising more sessions than there are slots, or more
pool than exists, must fail here rather than in production. If the tool cannot
read the new `tensor-split` key, extend it — that is in scope.

- [ ] **Step 4: Check the per-card arithmetic explicitly for the pinned 9B**

```bash
python3 -c "
card=11.0; weights=5.68/1.073741824; kv=81920*9.0/1024/1024; ctx=0.30; buf=0.80
used=weights+kv+ctx+buf
print(f'9B on one card: {used:.2f} of {card} GiB, margin {card-used:.2f}')
assert used < card, 'the pinned 9B does not fit one card'
"
```
Expected: ~7.1 of 11.0 GiB, margin ~3.9.

- [ ] **Step 5: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/config/router-presets.ini \
        llm/linux-turing-dual/config/profiles.d/router.conf
git commit -m "feat(llm): serve both models from one endpoint, one at a time

--models-max 1 is not a default worth inheriting: the 27B holds 15.3 of 22 GiB
and a second resident model would OOM several minutes into a load. Switching
tiers of one model is free because an alias resolves to the same weights;
switching models costs one reload from NVMe.

The 9B is pinned to a single card with tensor-split 1,0 because it fits in 11 GiB
and layer-splitting it would buy nothing but overhead. Cross-slot KV reuse stays
at 0.0 with the reason attached, so the next reader does not re-enable the thing
that crashed."
```

---

### Task 12: The service unit, and a real completion

**Files:**
- Create: `llm/linux-turing-dual/systemd/qwen-turing@.service`

**Interfaces:**
- Consumes: Tasks 9-11.
- Produces: `qwen-turing@router.service`, listening on `${QT_PORT}`, requiring
  an API key.

- [ ] **Step 1: Create the service user and the API key**

```bash
sudo useradd --system --home-dir /var/lib/qwen-turing --shell /usr/sbin/nologin qwen-turing 2>/dev/null || true
sudo install -d -o qwen-turing -g qwen-turing -m 0750 /var/lib/qwen-turing
umask 077; openssl rand -hex 32 | sudo tee /etc/qwen-turing.key >/dev/null
sudo chmod 600 /etc/qwen-turing.key && sudo chown root:root /etc/qwen-turing.key
```
The key is root-only and reaches the service via `LoadCredential`, never via a
unit-file `Environment=` line (which any local user can read from
`systemctl show`).

- [ ] **Step 2: Write the unit — respecting the CUDA sandboxing ceiling**

```ini
[Unit]
Description=Qwen inference node, dual Turing (profile %i)
Wants=network-online.target
After=network-online.target

[Service]
Type=exec
User=qwen-turing
Group=qwen-turing
WorkingDirectory=/var/lib/qwen-turing
Environment=QT_CONF_DIR=/opt/qwen-turing/etc
LoadCredential=apikey:/etc/qwen-turing.key
ExecStartPre=/opt/qwen-turing/bin/vram-guard.sh --min-total 19000 --min-per-card 8000
ExecStart=/opt/qwen-turing/bin/serve-router.sh %i

Restart=on-failure
RestartSec=10
# Loading ~15 GiB of weights is not a hang.
TimeoutStartSec=900
TimeoutStopSec=120
KillSignal=SIGINT

UMask=0027
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=true
ProtectSystem=full

# NOTE -- deliberately NOT set, and why. Each of these breaks CUDA on this host:
#   ProtectSystem=strict     blocks the NVIDIA driver's /proc and /sys access
#   PrivateDevices=true      hides /dev/nvidia*, so CUDA cannot initialise
#   DevicePolicy=closed      same
#   MemoryDenyWriteExecute=  breaks the CUDA JIT
# ProtectSystem=full is the strongest setting that leaves the stack working.

[Install]
WantedBy=multi-user.target
```

- [ ] **Step 3: Write `serve-router.sh`**

It sources `common.conf` and `site.sh`, calls `require_site || exit $?`, reads
the key from `${CREDENTIALS_DIRECTORY}/apikey`, and execs:

```bash
exec "${QT_LLAMA_DIR}/llama-server" \
  --models "${QT_ROUTER_PRESETS}" \
  --models-max "${QT_ROUTER_MAX_LOADED}" \
  --host "${QT_NODE_ADDR}" --port "${QT_PORT}" \
  --api-key "$(cat "${CREDENTIALS_DIRECTORY}/apikey")" \
  --no-webui \
  --metrics \
  --slot-save-path "" \
  --slot-prompt-similarity "${QT_SLOT_PROMPT_SIMILARITY}"
```

Verify each flag exists in the pinned build before trusting it:
`"${QT_LLAMA_DIR}/llama-server" --help | grep -E 'models-max|no-webui|metrics|slot-prompt-similarity|api-key'`.
If a flag is absent in this tag, adapt and record it — do not pass an unknown
flag and let the unit restart-loop.

- [ ] **Step 4: Install and start**

```bash
sudo install -m 0644 llm/linux-turing-dual/systemd/qwen-turing@.service /etc/systemd/system/
sudo install -d /opt/qwen-turing/etc /opt/qwen-turing/bin
sudo install -m 0644 llm/linux-turing-dual/config/router-presets.ini /opt/qwen-turing/etc/
sudo install -m 0644 llm/linux-turing-dual/config/common.conf /opt/qwen-turing/etc/
sudo install -m 0755 llm/linux-turing-dual/scripts/serve-router.sh /opt/qwen-turing/bin/
sudo systemctl daemon-reload
sudo systemctl enable --now qwen-turing@router
sleep 60; systemctl status qwen-turing@router --no-pager | head -20
```

- [ ] **Step 5: Verify a REAL completion — not just that the port is open**

```bash
KEY=$(sudo cat /etc/qwen-turing.key)
. llm/linux-turing-dual/scripts/site.sh && require_site
curl -sf -H "Authorization: Bearer $KEY" "http://${QT_NODE_ADDR}:${QT_PORT}/v1/models" | python3 -m json.tool | head -20
curl -sf -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  "http://${QT_NODE_ADDR}:${QT_PORT}/v1/chat/completions" \
  -d '{"model":"qwen3.8-27b","messages":[{"role":"user","content":"Reply with exactly: OK"}],"max_tokens":10}' \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["choices"][0]["message"]["content"])'
```
Expected: both ids listed; the completion prints `OK`. **"The port is open" is
not this test.**

- [ ] **Step 6: Verify auth actually denies**

```bash
curl -s -o /dev/null -w '%{http_code}\n' "http://${QT_NODE_ADDR}:${QT_PORT}/v1/models"
curl -s -o /dev/null -w '%{http_code}\n' -H "Authorization: Bearer wrong" "http://${QT_NODE_ADDR}:${QT_PORT}/v1/models"
```
Expected: `401` for both. A 200 here means the key is not being enforced —
stop and fix before the firewall step, not after.

- [ ] **Step 7: Verify the model switch works and is transparent**

```bash
time curl -sf -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  "http://${QT_NODE_ADDR}:${QT_PORT}/v1/chat/completions" \
  -d '{"model":"qwen3.5-9b","messages":[{"role":"user","content":"Reply with exactly: OK"}],"max_tokens":10}'
```
Expected: `OK`, with the first call slower (a reload). A second call to the same
id is fast. Record both timings in `docs/measured-ceilings.md` — this is the
"switch cost" the operator was told is seconds.

- [ ] **Step 8: Run the structural test now that the unit exists**

Run: `bash llm/linux-turing-dual/tests/test_structural.sh`
Expected: check 5 now runs and reports `unit uses ProtectSystem=full`.

- [ ] **Step 9: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/systemd/qwen-turing@.service \
        llm/linux-turing-dual/scripts/serve-router.sh \
        llm/linux-turing-dual/docs/measured-ceilings.md
git commit -m "feat(llm): serve the node under systemd, with the key off the unit file

The API key arrives via LoadCredential rather than Environment=, because any
local user can read the latter out of systemctl show. The VRAM guard runs as
ExecStartPre so a doomed start fails in a second instead of OOMing minutes into
a weight load.

The sandboxing stops at ProtectSystem=full and the unit carries the reasons for
every setting deliberately omitted, since each of them breaks CUDA and a future
hardening pass will otherwise tighten them back."
```

---

### Task 13: Measure what was only predicted

Nothing in this plan may claim a number this task has not produced.

**Files:**
- Modify: `llm/linux-turing-dual/docs/measured-ceilings.md`
- Modify: `llm/linux-qwen38/config/gpu-registry.json` (`measured_tps`)
- Modify: `llm/linux-turing-dual/config/model-artifacts.yaml` (`verified_max`)

**Interfaces:**
- Consumes: a running service from Task 12.
- Produces: measured tok/s, a verified context ceiling, and a seat cost.

- [ ] **Step 1: Measure single-stream throughput**

Run: `python3 llm/linux-qwen38/scripts/benchmark.py --server "http://${QT_NODE_ADDR}:${QT_PORT}" --model qwen3.8-27b --api-key "$KEY"`
Expected: a tok/s figure. Compare against the ~29.9 prediction. The project has
hit 79.8-80.0% of roofline on two other cards, so a result far off that is a
finding, not noise — investigate before recording.

- [ ] **Step 2: Verify the context ceiling AT LENGTH**

Run: `bash llm/linux-qwen38/scripts/measure-ceiling.sh --server "http://${QT_NODE_ADDR}:${QT_PORT}" --model qwen3.8-27b`
Expected: a needle retrieved at ~40k **actual** prompt tokens. "Allocates",
"starts" and "works at length" are three different claims and only the third
counts. Record all three numbers if they differ.

- [ ] **Step 3: Measure what a concurrent seat costs**

Run: `bash llm/linux-qwen38/scripts/measure-slot-frontier.sh --server "http://${QT_NODE_ADDR}:${QT_PORT}"`
Expected: the pool figure at `parallel = 2` and at `parallel = 3`. If 3 seats
fit, say so — the operator noted context sizes are adjustable later, and this is
the measurement that would justify raising them.

- [ ] **Step 4: Run two concurrent 40k sessions (acceptance criterion 4)**

Run: `python3 llm/linux-qwen38/scripts/bench-concurrency.py --server "http://${QT_NODE_ADDR}:${QT_PORT}" --model qwen3.8-27b-40k --sessions 2 --api-key "$KEY"`
Expected: both complete. **No `decode: failed to find free space in the KV
cache` and no `Context size has been exceeded`.** Either message means the pool
is over-subscribed and `c` must come down.

- [ ] **Step 5: Record the measurements, replacing the predictions**

Fill `measured_tps` in the registry with the real figures, set `verified_max` in
`model-artifacts.yaml` to what Step 2 actually retrieved, and write
`docs/measured-ceilings.md` with: what was measured, the numbers that *looked*
like the ceiling but were not, and what was never tested.

- [ ] **Step 6: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/docs/measured-ceilings.md \
        llm/linux-qwen38/config/gpu-registry.json \
        llm/linux-turing-dual/config/model-artifacts.yaml
git commit -m "docs(llm): replace the dual-Turing predictions with measurements

The roofline said ~30 tok/s for the 27B and ~87 for the 9B; these are what the
cards actually did. verified_max is now a number a real prompt reached rather
than null, and the numbers that looked like the ceiling without surviving a
prompt that filled it are recorded too, so they are not rediscovered later."
```

---

### Task 14: Firewall — Barrier 3, and only now

**Files:**
- Create: `llm/linux-turing-dual/ops/firewall.sh`

**Interfaces:**
- Consumes: a verified-working service.
- Produces: `ufw` default-deny with an explicit allowlist.

- [ ] **Step 1: Arm the dead-man switch BEFORE touching a rule**

```bash
sudo bash -c 'nohup sh -c "sleep 900; ufw --force disable" >/dev/null 2>&1 &'
echo "ufw will auto-disable in 15 minutes unless cancelled"
```
This is the whole reason a firewall change on a remote host is survivable.

- [ ] **Step 2: Write rules that leave Docker's chains alone**

`ufw` manipulates `INPUT`; Docker manipulates `FORWARD` directly. Do **not**
set `DEFAULT_FORWARD_POLICY=DROP` — this host carries Docker bridges, seven VLAN
bridges and libvirt networks.

```bash
sudo ufw --force reset
sudo ufw default deny incoming
sudo ufw default allow outgoing
sudo ufw allow from <internal-subnet> to any port 22 proto tcp
sudo ufw allow from <allowed-client-range> to any port "${QT_PORT}" proto tcp
sudo ufw allow from <internal-subnet> to any port "${QT_DASH_PORT}" proto tcp
```

- [ ] **Step 3: Verify SSH from a SECOND connection before dropping the first**

Open a new terminal and connect. Only once that new session works:
```bash
sudo ufw --force enable
sudo ufw status verbose
```
**Keep the original session open throughout.**

- [ ] **Step 4: Verify the service still answers and Docker still works**

```bash
curl -sf -o /dev/null -w '%{http_code}\n' -H "Authorization: Bearer $KEY" "http://${QT_NODE_ADDR}:${QT_PORT}/v1/models"
sudo iptables -S FORWARD | head -5
docker network ls
```
Expected: `200`; Docker's `FORWARD` chain intact; networks listed. If Docker
networking broke, the dead-man switch will restore access in under 15 minutes.

- [ ] **Step 5: Cancel the dead-man switch and persist**

```bash
sudo pkill -f 'sleep 900' || true
sudo ufw status | head -1
sudo systemctl enable ufw
```
Expected: `Status: active`.

- [ ] **Step 6: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/ops/firewall.sh
git commit -m "feat(llm): default-deny inbound, without cutting Docker's legs off

ufw runs after the reboot and after the service is proven, because enabling
default-deny on a host whose default was INPUT ACCEPT -- over SSH, minutes
before a reboot -- is how a machine becomes unreachable. A timed ufw disable is
armed before the first rule and cancelled only after a second independent
session has proven the SSH allow works.

DEFAULT_FORWARD_POLICY is left alone: Docker manages FORWARD itself and this
host carries seven VLAN bridges plus libvirt networks."
```

---

### Task 15: The stats dashboard

**Files:**
- Create: `llm/linux-turing-dual/scripts/collect-stats.py`
- Create: `llm/linux-turing-dual/scripts/dashboard.py`
- Create: `llm/linux-turing-dual/web/index.html`
- Create: `llm/linux-turing-dual/systemd/qwen-turing-dashboard.service`
- Test: `llm/linux-turing-dual/tests/test_collect_stats.py`

**Interfaces:**
- Consumes: llama.cpp `/metrics` and `nvidia-smi`.
- Produces: `GET /api/stats` returning
  `{"gpus":[{"index","name","util_pct","mem_used_mib","mem_total_mib","temp_c","power_w"}],
    "llama":{"prompt_tokens_total","generated_tokens_total","tokens_per_second","kv_cache_usage_ratio","slots_busy","slots_idle","model"},
    "ts"}`.

**Scope discipline:** one collector, one page, one listener, the same API key.
If this starts needing its own auth framework or a second listener, it has
regrown the complexity the operator explicitly removed.

- [ ] **Step 1: Write the failing test**

```python
# llm/linux-turing-dual/tests/test_collect_stats.py
import importlib.util, pathlib
SRC = pathlib.Path(__file__).resolve().parents[1] / "scripts" / "collect-stats.py"

def load():
    spec = importlib.util.spec_from_file_location("collect_stats", SRC)
    m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m); return m

PROM = """\
# HELP llamacpp:prompt_tokens_total Number of prompt tokens processed.
# TYPE llamacpp:prompt_tokens_total counter
llamacpp:prompt_tokens_total 123456
llamacpp:tokens_predicted_total 7890
llamacpp:predicted_tokens_seconds 29.7
llamacpp:kv_cache_usage_ratio 0.42
llamacpp:requests_processing 1
llamacpp:requests_deferred 0
"""

def test_parses_prometheus_counters():
    m = load(); d = m.parse_prometheus(PROM)
    assert d["llamacpp:prompt_tokens_total"] == 123456.0
    assert d["llamacpp:predicted_tokens_seconds"] == 29.7

def test_comments_are_not_metrics():
    m = load()
    assert not any(k.startswith("#") for k in m.parse_prometheus(PROM))

def test_parses_nvidia_smi_csv_for_every_card():
    m = load()
    csv = "0, NVIDIA GeForce RTX 2080 Ti, 47, 8192, 11264, 61, 142.5\n" \
          "1, NVIDIA GeForce RTX 2080 Ti, 39, 7100, 11264, 58, 130.0\n"
    gpus = m.parse_nvidia_csv(csv)
    assert len(gpus) == 2
    assert gpus[0]["util_pct"] == 47 and gpus[1]["mem_used_mib"] == 7100
    assert gpus[0]["name"] == "NVIDIA GeForce RTX 2080 Ti"

def test_missing_metrics_degrade_rather_than_crash():
    m = load()
    assert m.parse_prometheus("") == {}
    assert m.parse_nvidia_csv("") == []
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-turing-dual/tests/test_collect_stats.py -v`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement the two parsers plus a `collect()` that joins them**

```python
def parse_prometheus(text: str) -> dict[str, float]:
    """Prometheus exposition -> {name: value}. Comments are not metrics."""
    out: dict[str, float] = {}
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split()
        if len(parts) < 2:
            continue
        try:
            out[parts[0]] = float(parts[-1])
        except ValueError:
            continue
    return out


NVIDIA_FIELDS = ("index", "name", "utilization.gpu", "memory.used",
                 "memory.total", "temperature.gpu", "power.draw")


def parse_nvidia_csv(text: str) -> list[dict]:
    """One dict per card. An empty read is an empty list, never an exception."""
    gpus = []
    for line in text.splitlines():
        if not line.strip():
            continue
        f = [c.strip() for c in line.split(",")]
        if len(f) < 7:
            continue
        def num(v, cast=int):
            try:
                return cast(float(v))
            except ValueError:
                return None
        gpus.append({
            "index": num(f[0]), "name": f[1], "util_pct": num(f[2]),
            "mem_used_mib": num(f[3]), "mem_total_mib": num(f[4]),
            "temp_c": num(f[5]), "power_w": num(f[6], float),
        })
    return gpus
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-turing-dual/tests/test_collect_stats.py -v`
Expected: 4 passed

- [ ] **Step 5: Write the page and the server**

`dashboard.py` serves `web/index.html` and `GET /api/stats`, requires the same
bearer key, binds `${QT_NODE_ADDR}:${QT_DASH_PORT}`, uses only the standard
library (`http.server`, `urllib`), and returns 503 with a readable message when
the inference service is down rather than a traceback.

`web/index.html` is self-contained — no CDN, no external fonts. It shows: tokens
processed (prompt and generated), current tok/s, per-card utilisation / VRAM /
temperature / power as two gauges, KV-pool usage, slots busy vs idle, and which
model is resident. It polls `/api/stats` every 2 s.

- [ ] **Step 6: Verify end to end against the live service**

```bash
sudo install -m 0644 llm/linux-turing-dual/systemd/qwen-turing-dashboard.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now qwen-turing-dashboard
curl -sf -H "Authorization: Bearer $KEY" "http://${QT_NODE_ADDR}:${QT_DASH_PORT}/api/stats" | python3 -m json.tool
curl -s -o /dev/null -w '%{http_code}\n' "http://${QT_NODE_ADDR}:${QT_DASH_PORT}/api/stats"
```
Expected: JSON with **two** GPU entries and non-zero
`prompt_tokens_total` after Task 13's benchmarks; and `401` unauthenticated.

- [ ] **Step 7: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/scripts/collect-stats.py \
        llm/linux-turing-dual/scripts/dashboard.py \
        llm/linux-turing-dual/web/index.html \
        llm/linux-turing-dual/systemd/qwen-turing-dashboard.service \
        llm/linux-turing-dual/tests/test_collect_stats.py
git commit -m "feat(llm): show tokens and utilisation without a container stack

llama.cpp already publishes prompt tokens, generated tokens, tok/s and KV-pool
usage, so the dashboard is one collector, one page and the standard library
rather than Prometheus plus Grafana plus an exporter. It reuses the inference
API key instead of growing an auth story of its own.

Both parsers degrade to empty rather than raising, because a stats page that
500s when the model is reloading is worse than one that shows a gap."
```

---

### Task 16: Documentation and the README row

**Files:**
- Create: `llm/linux-turing-dual/README.md`
- Modify: `llm/README.md`

- [ ] **Step 1: Write the node README as measured, not as intended**

Cover: what the node serves and at what verified context; the VRAM budget table
with the *measured* margin; the switch cost measured in Task 12; the two
multi-GPU bugs that had to be fixed and why a two-card host hits both; the
sandboxing ceiling; the campus-facing-without-TLS decision and its residual
risk; and the three-card path.

- [ ] **Step 2: Add the fourth row to `llm/README.md`**

```markdown
| two or more consumer NVIDIA cards | [`linux-turing-dual/README.md`](linux-turing-dual/README.md) | `linux-turing-dual/scripts/install-node.sh` |
```

- [ ] **Step 3: Verify the leak guard on the finished tree**

```bash
cd /tmp/wt-turing
QT_LEAK_PATTERNS='<the real address>|<the real hostname>' \
  bash llm/linux-turing-dual/tests/test_structural.sh
git grep -niE '192\.168\.|128\.120\.' -- llm docs || echo "clean"
```
Expected: `OK -- all structural checks passed` and `clean`.

- [ ] **Step 4: Run the full no-accelerator suite one final time**

```bash
cd /tmp/wt-turing
bash llm/linux-turing-dual/tests/test_structural.sh
python3 -m pytest llm/linux-turing-dual/tests/ llm/linux-qwen38/tests/test_select_quant_devices.py -q
python3 llm/linux-qwen38/tests/check_presets.py --config llm/linux-turing-dual/config/router-presets.ini
```
Expected: all pass.

- [ ] **Step 5: Verify the service survives a reboot (acceptance criterion 6)**

```bash
sudo systemctl reboot
# reconnect, then:
systemctl is-active qwen-turing@router qwen-turing-dashboard
curl -sf -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  "http://${QT_NODE_ADDR}:${QT_PORT}/v1/chat/completions" \
  -d '{"model":"qwen3.8-27b","messages":[{"role":"user","content":"Reply with exactly: OK"}],"max_tokens":10}'
sudo ufw status | head -1
df -h /
```
Expected: both units `active`; the completion returns `OK`; `ufw` active; root
below 50%. **This is the last acceptance criterion and it is unattended-start,
so it must be a real reboot, not a `systemctl restart`.**

- [ ] **Step 6: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/README.md llm/README.md
git commit -m "docs(llm): document the dual-Turing node as built and measured

Records the two defects a second card exposes -- both tools read nvidia-smi's
first row -- so the next multi-GPU node does not rediscover them, and the
sandboxing ceiling with the reason each omitted setting is omitted.

Also records the decisions the operator made against advice, with the residual
risk stated once: a campus-facing listener without TLS carries the API key in
cleartext. A later reader should be able to see that this was chosen, not
overlooked."
```

---

## Self-Review

**Spec coverage:**

| spec section | task |
|---|---|
| §0 new node directory | 3, 16 |
| §1 host audit + reboot safety | 5, 7 |
| §2 Tier A cleanup | 5 |
| §2 Tier B / C, accounts, Docker | 6 |
| §2.0 Barrier 1 (DKMS re-verify) | 5 Step 6, 7 Step 2 |
| §2.0 Barrier 2 (kernel fallback) | 5 Step 2 item 5, 7 Step 6 |
| §2.0 Barrier 3 (firewall after reboot) | 14 |
| §2.0 Barrier 4 (mdstat inside delete) | 6 Step 4 |
| §3 llama.cpp sm_75, vLLM rejected | 9, 16 |
| §4 multi-GPU layout, VRAM budget, KV | 11, 13 |
| §5 roster, switching, shared-context split | 11, 12 |
| §6 API key, sandboxing ceiling, ufw | 12, 14 |
| §7 dashboard | 15 |
| §8 network addressing | 3 (site.conf), 14 |
| §9 tooling fixes 1-2 | 1, 2 |
| §9 tooling fixes 3-5 | 8, 11 |
| §9 fixes 6-7 (leak guard, CI) | 3, 4 |
| §10 acceptance criteria 1-9 | 7, 12, 13, 16 |
| §11 three-card path | 16 |

**Gap found and closed:** §8's NIC tuning (ring buffers, offloads, irqbalance)
had no task. It is deliberately **not** given one — the spec itself records that
1 GbE buys zero inference performance here, so adding a task would be work the
spec argues against. Task 16 Step 1 documents that decision instead.

**Placeholder scan:** no unresolved-marker tokens of any kind. `<node-addr>`, `<nvme-array>`,
`<pinned-tag>`, `<sha-27b>` and `<internal-subnet>` are deliberate site values,
each resolved by a named step (Task 3, Task 9 Step 1, Task 10 Step 2).

**Type consistency:** `parse_device_totals` / `device_budget` /
`per_card_ceiling` (Task 1) are used consistently. `vram-guard.sh`'s
`--min-total` / `--min-per-card` and exits 0/69/75 match between Task 2, Task 9
Step 5 and Task 12's `ExecStartPre`. `require_site` returns 78 and is always
called as `require_site || exit $?` (Tasks 3, 10, 12). `parse_prometheus` /
`parse_nvidia_csv` match between Task 15's test and implementation.
`QT_*` naming is used throughout for the new node; `QWEN38_*` appears only where
the shared 4090 toolkit is modified.
