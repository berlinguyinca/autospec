#!/usr/bin/env bash
# scripts/explore-specialist-scan.sh — domain-specialist roster DISCOVERY
# (Issue E1, spec: docs/specs/2026-06-15-autospec-explore-discovery-enhance.md
# §"Domain-specialist researchers" → "Roster discovery").
#
# Step 1 (this script, deterministic, NO LLM): scan dependency manifests,
# README/AGENTS.md keywords, and directory taxonomy against a small domain
# lexicon → a ranked list of candidate domains, each with file:line evidence.
# Never a bare guess: a domain only appears if a concrete signal cited it.
#
# Step 2 (LLM roster proposal) is the explore-orchestrator's (SKILL-prose)
# responsibility. This script provides the seam: given the deterministic
# signals it produces the `suggested_specialists[]` roster, either from the
# orchestrator's LLM output (via AUTOSPEC_SPECIALIST_LLM_STUB_OUTPUT, the same
# stub channel the LLM researchers use) or, absent that, a deterministic
# persona derived from the top-ranked domains. Personas are repo-evidence-only;
# no external persona is injected (trust boundary, spec §Guardrails).
#
# Output: the roster is cached to .autospec/explore-specialists.json under
# schemas/autospec-explore-specialists.schema.json and reused idempotently on
# re-invocation. The same JSON is echoed to stdout.
#
# Generic repo (no domain detected) → empty roster; the explore loop runs
# exactly as today (spec §Guardrails: "degrades gracefully").
#
# Out of scope (Issue E2 #1083): the --specialists-mode / --num-specialists /
# --specialists flags, AskUserQuestion confirm, and dispatch as
# source=specialist:<slug>. This script only DISCOVERS and CACHES the roster.

set -u

REPO_ROOT="${AUTOSPEC_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
NUM_SPECIALISTS="${AUTOSPEC_NUM_SPECIALISTS:-3}"
ROSTER_PATH="$REPO_ROOT/.autospec/explore-specialists.json"

# --force re-derives even when a cache exists (idempotent default reuses it).
FORCE=0
for arg in "$@"; do
    case "$arg" in
        --force) FORCE=1 ;;
    esac
done

cd "$REPO_ROOT" 2>/dev/null || { echo '{"schema_version":1,"domains":[],"suggested_specialists":[]}'; exit 0; }

# ── Idempotent reuse ──────────────────────────────────────────────
if [ "$FORCE" -ne 1 ] && [ -f "$ROSTER_PATH" ]; then
    if command -v jq >/dev/null 2>&1 && jq -e . "$ROSTER_PATH" >/dev/null 2>&1; then
        cat "$ROSTER_PATH"
        exit 0
    fi
fi

if ! command -v python3 >/dev/null 2>&1; then
    mkdir -p "$REPO_ROOT/.autospec" 2>/dev/null || true
    printf '%s\n' '{"schema_version":1,"domains":[],"suggested_specialists":[]}' | tee "$ROSTER_PATH" 2>/dev/null || true
    exit 0
fi

# ── Deterministic signal scan + roster derivation ─────────────────
# All scanning happens in python3 for portable file:line evidence on bash 3.2.
export _AUTOSPEC_REPO_ROOT="$REPO_ROOT"
export _AUTOSPEC_NUM_SPECIALISTS="$NUM_SPECIALISTS"
export _AUTOSPEC_LLM_STUB="${AUTOSPEC_SPECIALIST_LLM_STUB_OUTPUT:-}"

ROSTER_JSON="$(python3 - <<'PY'
import json, os, re

root = os.environ["_AUTOSPEC_REPO_ROOT"]
try:
    num = max(0, min(6, int(os.environ.get("_AUTOSPEC_NUM_SPECIALISTS", "3"))))
except ValueError:
    num = 3
stub = os.environ.get("_AUTOSPEC_LLM_STUB", "")

# Small domain lexicon: domain -> (regex of signal tokens, persona, lens).
# Tokens are matched case-insensitively against manifest deps, README/AGENTS
# keywords, and directory names. Evidence is always a real file:line.
LEXICON = {
    "trading": {
        "tokens": r"\b(ccxt|backtrader|zipline|alpaca|quantlib|ta-lib|talib|"
                  r"backtest(?:ing)?|order[- ]?book|market[- ]?data|ohlcv|"
                  r"exchange[- ]?api|trading|brokerage|portfolio)\b",
        "persona": "Quantitative trading strategist",
        "lens": "missing risk controls, order-execution correctness, and "
                "market-data integrity",
    },
    "healthcare": {
        "tokens": r"\b(hl7|fhir|hipaa|dicom|ehr|emr|patient|clinical|"
                  r"healthcare|phi|icd-?10)\b",
        "persona": "Healthcare compliance reviewer",
        "lens": "PHI handling, HIPAA boundaries, and clinical-data integrity",
    },
    "payments": {
        "tokens": r"\b(stripe|braintree|paypal|pci|pci-dss|payment|checkout|"
                  r"invoice|billing|ledger|chargeback)\b",
        "persona": "Payments reliability engineer",
        "lens": "idempotency, reconciliation, and PCI trust boundaries",
    },
    "ml": {
        "tokens": r"\b(pytorch|torch|tensorflow|keras|scikit-learn|sklearn|"
                  r"huggingface|transformers|xgboost|lightgbm|inference|"
                  r"training[- ]?loop|model[- ]?registry)\b",
        "persona": "ML systems reviewer",
        "lens": "training/serving skew, data leakage, and reproducibility",
    },
    "security": {
        "tokens": r"\b(oauth|jwt|bcrypt|argon2|crypto|cryptography|tls|mtls|"
                  r"vault|secrets[- ]?manager|rbac|authz|authentication)\b",
        "persona": "Application security advisor",
        "lens": "authz boundaries, secret handling, and injection surfaces",
    },
    "infra": {
        "tokens": r"\b(kubernetes|k8s|terraform|helm|docker[- ]?compose|"
                  r"ansible|pulumi|cloudformation|prometheus|grafana)\b",
        "persona": "Infrastructure reliability engineer",
        "lens": "blast radius, rollout safety, and observability gaps",
    },
}

# Files to scan for signal tokens. Manifests + docs; directory taxonomy is
# handled separately so we cite the directory entry itself.
MANIFESTS = [
    "package.json", "requirements.txt", "pyproject.toml", "go.mod",
    "Cargo.toml", "Gemfile", "pom.xml", "build.gradle",
]
DOCS = ["README.md", "AGENTS.md", "README.rst"]

def rel(path):
    return os.path.relpath(path, root)

# Gather scan targets: explicit manifests/docs at root + any *.csproj anywhere
# shallow + directory names one level down.
scan_files = []
for name in MANIFESTS + DOCS:
    p = os.path.join(root, name)
    if os.path.isfile(p):
        scan_files.append(p)

# *.csproj (shallow walk, skip vcs/vendor dirs)
SKIP_DIRS = {".git", "node_modules", "vendor", ".venv", "venv", "target",
             "dist", "build", ".autospec"}
dir_names = []
try:
    for entry in sorted(os.listdir(root)):
        full = os.path.join(root, entry)
        if os.path.isdir(full) and entry not in SKIP_DIRS and not entry.startswith("."):
            dir_names.append(entry)
        elif os.path.isfile(full) and entry.endswith(".csproj"):
            scan_files.append(full)
except OSError:
    pass

# domain -> list of evidence dicts {file,line,match}
hits = {d: [] for d in LEXICON}

def record(domain, file_rel, line_no, match_text):
    # Cap evidence per domain to keep the roster small and deterministic.
    if len(hits[domain]) >= 8:
        return
    hits[domain].append({
        "file": file_rel,
        "line": int(line_no),
        "match": match_text[:120],
    })

# Scan file contents line-by-line for token matches.
for fpath in scan_files:
    try:
        with open(fpath, "r", encoding="utf-8", errors="replace") as fh:
            for i, line in enumerate(fh, start=1):
                low = line.lower()
                for domain, spec in LEXICON.items():
                    m = re.search(spec["tokens"], low)
                    if m:
                        record(domain, rel(fpath), i, line.strip())
    except OSError:
        continue

# Directory taxonomy: a top-level dir whose name matches a domain token is a
# signal. Cite the directory as file:line (line 0 -> 1 to satisfy schema).
for d in dir_names:
    low = d.lower()
    for domain, spec in LEXICON.items():
        if re.search(spec["tokens"], low):
            # Use the directory path with a sentinel line of 1.
            record(domain, d + "/", 1, "directory: " + d)

# Rank domains by distinct evidence count (score), then name for stability.
ranked = []
for domain, ev in hits.items():
    if not ev:
        continue
    ranked.append({"name": domain, "score": len(ev), "evidence": ev})
ranked.sort(key=lambda x: (-x["score"], x["name"]))

# ── Roster proposal ───────────────────────────────────────────────
specialists = []
parsed_stub = None
if stub.strip():
    m = re.search(r'\[.*\]|\{.*\}', stub, re.DOTALL)
    if m:
        try:
            parsed_stub = json.loads(m.group(0))
        except Exception:
            parsed_stub = None

def clean_specialist(item, allowed_evidence_index):
    if not isinstance(item, dict):
        return None
    slug = str(item.get("slug", "")).strip().lower()
    slug = re.sub(r"[^a-z0-9]+", "-", slug).strip("-")
    persona = str(item.get("persona", "")).strip()
    lens = str(item.get("lens", "")).strip()
    why = str(item.get("why", "")).strip()
    evidence = str(item.get("evidence", "")).strip()
    if not (slug and persona and lens and why and evidence):
        return None
    return {"slug": slug, "persona": persona, "lens": lens,
            "why": why, "evidence": evidence}

if parsed_stub is not None:
    arr = parsed_stub
    if isinstance(parsed_stub, dict):
        arr = parsed_stub.get("suggested_specialists", [])
    if isinstance(arr, list):
        for item in arr:
            c = clean_specialist(item, None)
            if c:
                specialists.append(c)
else:
    # Deterministic fallback: derive one specialist per top-ranked domain from
    # repo evidence only. No external persona injected (trust boundary).
    for dom in ranked[:num]:
        spec = LEXICON[dom["name"]]
        first = dom["evidence"][0]
        specialists.append({
            "slug": dom["name"] + "-specialist",
            "persona": spec["persona"],
            "lens": spec["lens"],
            "why": "Repo signals indicate a {} domain; a specialist lens "
                   "surfaces domain-specific gaps the universal researchers "
                   "miss.".format(dom["name"]),
            "evidence": "{}:{} ({})".format(first["file"], first["line"],
                                            first["match"]),
        })

specialists = specialists[:num]

roster = {
    "schema_version": 1,
    "domains": ranked,
    "suggested_specialists": specialists,
}
print(json.dumps(roster, indent=2))
PY
)"

PY_RC=$?
if [ "$PY_RC" -ne 0 ] || [ -z "$ROSTER_JSON" ]; then
    ROSTER_JSON='{"schema_version":1,"domains":[],"suggested_specialists":[]}'
fi

# ── Cache (idempotent) + echo ─────────────────────────────────────
mkdir -p "$REPO_ROOT/.autospec" 2>/dev/null || true
printf '%s\n' "$ROSTER_JSON" > "$ROSTER_PATH" 2>/dev/null || true
printf '%s\n' "$ROSTER_JSON"
