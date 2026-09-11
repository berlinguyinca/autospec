#!/usr/bin/env bash
# check-ci-concurrency-group.sh — issue #4199: enumerate the CI
# concurrency-group defect population.
#
# The defect class: "a shared concurrency group on a branch that receives
# frequent merges silently discards verification" (a new merge cancels the run
# before it, so the commit carries no CI result). A shared group is harmless at
# one merge an hour and destroys verification at nine merges in three minutes.
#
# Method: parse each workflow as YAML (python3 + PyYAML) and read
# concurrency.group / cancel-in-progress. The fixed per-commit group in
# .github/workflows/rust.yml carries 8-12 comment lines between `concurrency:`
# and `group:`, so a `grep -A3` context window never reaches it — and a
# detector that under-matches reports the population as clean (issue #4199).
# Parsing beats grepping whenever the artefact has a parser.
#
# A fix is not finished when the reported instance stops failing: a defect
# class has a population. When this defect is fixed in one repository, run
# this script on every repository, workflow and template that could carry the
# same shape and record the enumeration in the fix ("checked repos A, B, C;
# B also affected, filed #N") so the absence of a sibling report means
# "checked and clean", not "never looked". Templates installed into consumer
# repositories (see skills/autospec-shared/scripts/install-doc-drift-workflow.sh)
# belong at the front of the enumeration — a defect in a template propagates to
# every consumer and fixing the original fixes none of them.
#
# Classification:
#   class=per-commit  group carries a per-sha discriminator (github.sha)
#   class=shared      group is shared per ref (no per-sha element)
#   class=none        no concurrency block (n/a)
# cancel-in-progress: yes / no / expr (a ${{ }} expression — the naive form
#   proven unreliable by six cancelled runs, see .github/workflows/rust.yml)
#
# Verdict:
#   ok                per-commit; or shared without cancel-in-progress; or
#                     shared+cancel on a pull-request-only workflow (a
#                     superseded PR run genuinely is worthless — the case the
#                     shared group exists for)
#   DEFECT-CANDIDATE  shared group + cancel-in-progress + a push trigger
#   na                no concurrency block
#   ERROR             workflow could not be parsed — fail-closed, never clean
#
# Known-answer assertions (--expect) run against the parsed population, in
# both directions, before the sweep is trusted: a mismatch is a detector
# failure, not a workflow finding.
#
# Output (stdout, one line per workflow, plus one per assertion):
#   <relpath>: class=<none|per-commit|shared> cancel=<yes|no|expr|absent> push=<yes|no> verdict=<ok|DEFECT-CANDIDATE|na|ERROR>
#   <relpath>: per-commit=<True|False> expected=<True|False> -> OK|FAIL
#
# Usage:
#   check-ci-concurrency-group.sh [WORKFLOW_DIR] [--expect REL=True|False]... [--list]
#
#   WORKFLOW_DIR  workflow directory to sweep (default: .github/workflows)
#   --expect      known-answer assertion; the class may also be given as
#                 per-commit|shared|none
#   --list        audit mode: emit the sweep, always exit 0
#
# Exit codes: 0 clean (all expectations met); 1 defect candidate, failed
# expectation, or unparseable workflow; 2 usage error.

set -euo pipefail

usage() {
    sed -n 's/^# \{0,1\}//p' "$0" | head -n 55
}

die_usage() {
    printf 'check-ci-concurrency-group: %s\n' "$1" >&2
    exit 2
}

WORKFLOW_DIR=""
LIST=0
EXPECTS=()

while [ "$#" -gt 0 ]; do
    case "$1" in
        --expect)
            [ "$#" -ge 2 ] || die_usage "--expect requires a REL=True|False value"
            case "$2" in
                *=*) EXPECTS+=("$2") ;;
                *) die_usage "--expect value must be REL=True|False (got: $2)" ;;
            esac
            shift 2
            ;;
        --list) LIST=1; shift ;;
        -h|--help) usage; exit 0 ;;
        -*) die_usage "unknown option: $1" ;;
        *)
            if [ -n "$WORKFLOW_DIR" ]; then
                die_usage "only one WORKFLOW_DIR is accepted (got: $1)"
            fi
            WORKFLOW_DIR="$1"
            shift
            ;;
    esac
done

[ -n "$WORKFLOW_DIR" ] || WORKFLOW_DIR=".github/workflows"
if [ ! -d "$WORKFLOW_DIR" ]; then
    die_usage "workflow directory not found: $WORKFLOW_DIR"
fi

if ! command -v python3 >/dev/null 2>&1; then
    die_usage "python3 is required (with PyYAML) to parse the workflows"
fi

# Parse each workflow and emit TSV: rel, class, cancel, push, verdict, detail.
# A parse failure is a row (class=unparseable, verdict=ERROR), never a
# silent "no concurrency block" — an under-matching detector reports the
# population as clean.
if ! SWEEP="$(python3 - "$WORKFLOW_DIR" <<'PY'
import os
import sys

try:
    import yaml
except ImportError:
    sys.stderr.write("check-ci-concurrency-group: python3 PyYAML is required\n")
    sys.exit(2)

workflow_dir = sys.argv[1]


def push_present(triggers):
    if isinstance(triggers, dict):
        return "push" in triggers
    if isinstance(triggers, list):
        return "push" in triggers
    return False


rows = []
for name in sorted(os.listdir(workflow_dir)):
    path = os.path.join(workflow_dir, name)
    if not (name.endswith(".yml") or name.endswith(".yaml")):
        continue
    if not os.path.isfile(path):
        continue
    try:
        with open(path, "r", encoding="utf-8") as fh:
            data = yaml.safe_load(fh)
    except (yaml.YAMLError, OSError) as exc:
        rows.append((name, "unparseable", "absent", "no", "ERROR",
                     " ".join(str(exc).split())))
        continue
    if not isinstance(data, dict):
        rows.append((name, "none", "absent", "no", "na",
                     "workflow file is not a mapping"))
        continue

    # PyYAML reads the unquoted `on:` key as boolean True (YAML 1.1);
    # accept both spellings.
    triggers = data.get("on", data.get(True))
    push = push_present(triggers)

    conc = data.get("concurrency")
    group = conc.get("group") if isinstance(conc, dict) else None
    if not isinstance(group, str) or not group.strip():
        rows.append((name, "none", "absent", "yes" if push else "no", "na", ""))
        continue

    # The per-commit discriminator is a per-sha element in the group; a
    # group built only from github.ref is shared across the branch.
    klass = "per-commit" if "github.sha" in group else "shared"
    cancel = conc.get("cancel-in-progress", False)
    if cancel is True:
        cancel = "yes"
    elif cancel is False:
        cancel = "no"
    elif isinstance(cancel, str):
        cancel = "expr"  # ${{ }} — the form six cancelled runs proved unreliable
    else:
        cancel = "no"

    if klass == "per-commit":
        verdict = "ok"
    elif cancel in ("yes", "expr") and push:
        verdict = "DEFECT-CANDIDATE"
    else:
        verdict = "ok"
    rows.append((name, klass, cancel, "yes" if push else "no", verdict, ""))

for row in rows:
    sys.stdout.write("\t".join(row) + "\n")
PY
)"; then
    printf 'check-ci-concurrency-group: workflow parse failed for %s\n' "$WORKFLOW_DIR" >&2
    exit 2
fi

status=0

while IFS=$'\t' read -r name klass cancel push verdict detail; do
    [ -n "$name" ] || continue
    line="$(printf '%s: class=%s cancel=%s push=%s verdict=%s' "$name" "$klass" "$cancel" "$push" "$verdict")"
    if [ -n "$detail" ]; then
        line="${line} (${detail})"
    fi
    printf '%s\n' "$line"
    case "$verdict" in
        DEFECT-CANDIDATE)
            printf '%s: shared concurrency group + cancel-in-progress on a push-triggered workflow silently cancels the previous merge verification run (issue #4199)\n' "$name" >&2
            status=1
            ;;
        ERROR)
            printf '%s: workflow could not be parsed - fail-closed, the population is NOT clean\n' "$name" >&2
            status=1
            ;;
    esac
done <<< "$SWEEP"

for spec in ${EXPECTS[@]+"${EXPECTS[@]}"}; do
    rel="${spec%%=*}"
    raw="${spec#*=}"
    case "$raw" in
        True|true|per-commit) want="True" ;;
        False|false|shared|none) want="False" ;;
        *) die_usage "--expect value must be True|False (or per-commit|shared|none), got: $raw" ;;
    esac
    actual="$(printf '%s\n' "$SWEEP" | awk -F'\t' -v n="$rel" '$1 == n { print $2; exit }')"
    if [ -z "$actual" ]; then
        printf '%s: per-commit=? expected=%s -> FAIL (workflow not found in %s)\n' "$rel" "$want" "$WORKFLOW_DIR"
        status=1
        continue
    fi
    if [ "$actual" = "unparseable" ]; then
        printf '%s: per-commit=? expected=%s -> FAIL (workflow could not be parsed)\n' "$rel" "$want"
        status=1
        continue
    fi
    if [ "$actual" = "per-commit" ]; then
        actual_pc="True"
    else
        actual_pc="False"
    fi
    if [ "$actual_pc" = "$want" ]; then
        printf '%s: per-commit=%s expected=%s -> OK\n' "$rel" "$actual_pc" "$want"
    else
        printf '%s: per-commit=%s expected=%s -> FAIL\n' "$rel" "$actual_pc" "$want"
        status=1
    fi
done

if [ "$LIST" -eq 1 ]; then
    exit 0
fi
exit "$status"
