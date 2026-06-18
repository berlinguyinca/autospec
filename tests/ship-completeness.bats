#!/usr/bin/env bats
# tests/ship-completeness.bats — guard that every script a skill surface invokes
# via ${AUTOSPEC_SCRIPTS_DIR} is actually shipped by the installers, and that no
# bare repo-relative shell-script invocation remains in any skill surface file.
#
# Rationale (#556): repo-root scripts ship to ~/.autospec/scripts via install.sh's
# copy_repo_scripts() glob + skills/autospec-shared/scripts via copy_shared_scripts().
# Per-skill runtime helpers (skills/<skill>/scripts/) ship via copy_runtime_skill_scripts()'s
# explicit src->dest manifest (#985). A runtime reference to a script that the installers
# never copy would be missing in a target repo. This guard catches such drift at test time.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"

# Skill surface files that may reference helper scripts at runtime: per-skill trios,
# the autospec orchestrator surface, and any prompt files dispatched to subagents.
surface_files() {
  {
    ls "$REPO_ROOT"/skills/*/SKILL.md 2>/dev/null
    ls "$REPO_ROOT"/skills/*/codex/prompt.md 2>/dev/null
    ls "$REPO_ROOT"/skills/*/opencode/agent.md 2>/dev/null
    ls "$REPO_ROOT"/skills/*/prompts/*.md 2>/dev/null
    find "$REPO_ROOT/skills/autospec" -type f -name '*.md' 2>/dev/null
  } | sort -u
}

# The set of relative paths (under ~/.autospec/scripts) the installers ship: repo-root
# scripts/ globs (.sh/.mjs/.ps1, excluding install-time-only scripts/lib/) land at the
# top level, while the entire skills/autospec-shared/scripts/ tree is copied preserving
# its subdirectory layout. We record both the flat relative path and the basename so a
# reference can be matched whether or not it carries a subdirectory prefix.
shippable_paths() {
  {
    # Repo-root scripts/*.{sh,mjs,ps1} ship flat (basename only); exclude scripts/lib/.
    ls "$REPO_ROOT"/scripts/*.sh "$REPO_ROOT"/scripts/*.mjs "$REPO_ROOT"/scripts/*.ps1 2>/dev/null \
      | while read -r f; do basename "$f"; done
    # Shared scripts ship preserving their tree under ~/.autospec/scripts/.
    if [ -d "$REPO_ROOT/skills/autospec-shared/scripts" ]; then
      ( cd "$REPO_ROOT/skills/autospec-shared/scripts" && \
        find . -type f \( -name '*.sh' -o -name '*.mjs' -o -name '*.ps1' \) \
          | sed 's#^\./##' )
    fi
    # Per-skill runtime helpers ship flat (basename) via copy_runtime_skill_scripts()'s
    # explicit "<src>::<dest>" manifest in install.sh. Read the manifest's declared
    # destinations so this guard tracks what the installer actually copies (#985). The
    # references being checked come from skill surface files, not from install.sh, so
    # this is not a self-consistent fixture.
    if [ -f "$REPO_ROOT/install.sh" ]; then
      grep -oE 'skills/[A-Za-z0-9_./-]+\.(sh|mjs|ps1)::[A-Za-z0-9_.-]+\.(sh|mjs|ps1)' \
        "$REPO_ROOT/install.sh" \
        | sed -E 's#^.*::##'
    fi
  } | sort -u
}

# Every ${AUTOSPEC_SCRIPTS_DIR...}/<path> reference, reduced to the path after the var.
referenced_paths() {
  surface_files | while read -r f; do
    grep -hoE '\$\{AUTOSPEC_SCRIPTS_DIR[^}]*\}/[A-Za-z0-9_./-]+\.(sh|mjs|ps1)' "$f" 2>/dev/null
  done \
    | sed -E 's#\$\{AUTOSPEC_SCRIPTS_DIR[^}]*\}/##' \
    | sort -u
}

# A referenced path is shipped when it matches a shippable relative path exactly, or
# (for a bare basename reference) matches the basename of any shipped file.
is_shipped() {
  ref="$1"; shippable="$2"
  # Exact relative-path match (handles subdir refs precisely).
  printf '%s\n' "$shippable" | grep -qxF "$ref" && return 0
  # Bare basename reference (no slash): allow matching any shipped file's basename.
  case "$ref" in
    */*) return 1 ;;
    *)
      printf '%s\n' "$shippable" | while read -r s; do
        [ "$(basename "$s")" = "$ref" ] && echo MATCH
      done | grep -q MATCH
      ;;
  esac
}

@test "every \${AUTOSPEC_SCRIPTS_DIR} script reference is shipped by the installers" {
  shippable="$(shippable_paths)"
  missing=""
  while read -r ref; do
    [ -n "$ref" ] || continue
    if ! is_shipped "$ref" "$shippable"; then
      missing="$missing $ref"
    fi
  done <<< "$(referenced_paths)"
  if [ -n "$missing" ]; then
    echo "Unshipped \${AUTOSPEC_SCRIPTS_DIR} references:$missing" >&2
    echo "These scripts are invoked at runtime but not copied by install.sh." >&2
    false
  fi
}

@test "no bare repo-relative shell/mjs script invocation in skill surfaces" {
  # Allow-listed, intentionally repo-relative invocations (NOT shipped to
  # ~/.autospec/scripts; they run from a repo checkout, not the installed scripts dir):
  #   - skills/autospec-test/scripts/run-gate.sh: the autospec-test target-repo gate, an
  #     opt-in that runs from the target repo checkout.
  #   - scripts/validate.sh: the lock-step / full-suite validator. In autospec-run /
  #     autospec / autospec-sweep it is "the repo-standard full suite ... when present",
  #     i.e. the TARGET repo's own validate.sh; in autospec-explore it is this repo's
  #     own lock-step gate run from the autospec checkout. It is never copied to
  #     ~/.autospec/scripts, so a ${AUTOSPEC_SCRIPTS_DIR} rewrite would be incorrect.
  offenders="$(
    surface_files | while read -r f; do
      grep -nE '(bash[[:space:]]+|\./|[^/A-Za-z._-])scripts/[A-Za-z0-9_./-]+\.(sh|mjs)' "$f" 2>/dev/null \
        | sed "s#^#${f}:#"
    done \
      | grep -v 'skills/autospec-test/scripts/run-gate.sh' \
      | grep -vE '(bash[[:space:]]+|\./)scripts/validate\.sh' \
      || true
  )"
  if [ -n "$offenders" ]; then
    echo "Bare repo-relative script invocations found (must use \${AUTOSPEC_SCRIPTS_DIR}):" >&2
    echo "$offenders" >&2
    false
  fi
}

# Regression guard for the lib/explore-research ship gap: copy_repo_scripts() is
# maxdepth-1, so NOTHING under scripts/lib/ or scripts/explore-research/ ships via the
# glob. Runtime libs that installed repo-root scripts source via $SCRIPT_DIR/lib/<x>
# (e.g. autospec-explore.sh: lib/autospec-loop.sh) must instead ship via the dedicated
# copy_runtime_subdirs() step. The references are extracted from scripts/*.sh and the
# shipment list from install.sh, so this is a true cross-check, not a self-consistent
# fixture (cf. the green-suite-broken-install gap that let this bug ship).
@test "runtime \$SCRIPT_DIR/lib sources are shipped by copy_runtime_subdirs" {
  # Runtime libs that installed repo-root scripts source/exec via $SCRIPT_DIR/lib/<x>.sh.
  referenced_libs="$(grep -rhoE '\$\{?SCRIPT_DIR\}?/lib/[A-Za-z0-9_.-]+\.sh' "$REPO_ROOT"/scripts/*.sh 2>/dev/null \
    | sed -E 's#.*/lib/##' | sort -u)"

  # What copy_runtime_subdirs() declares it ships (the runtime_libs list in install.sh).
  shipped_libs="$(sed -n 's/^[[:space:]]*runtime_libs="\(.*\)"/\1/p' "$REPO_ROOT/install.sh" | tr ' ' '\n' | sort -u)"

  missing=""
  for lib in $referenced_libs; do
    echo "$shipped_libs" | grep -qx "$lib" || missing="$missing $lib"
  done
  if [ -n "$missing" ]; then
    echo "Runtime \$SCRIPT_DIR/lib sources not shipped by copy_runtime_subdirs():$missing" >&2
    echo "Add each to the runtime_libs list in install.sh copy_runtime_subdirs()." >&2
    false
  fi

  # scripts/explore-research/ (the researcher dir resolved at $SCRIPT_DIR/explore-research)
  # must be shipped as a directory by the same step.
  grep -q 'scripts/explore-research' "$REPO_ROOT/install.sh" || {
    echo "install.sh copy_runtime_subdirs() does not ship scripts/explore-research/" >&2
    false
  }
}

# --- autospec-doc ES-module closure (the doc-orchestrator regression) ----------
# The orchestrator's relative `./*.mjs` imports must ALL be shipped into the
# subtree, else it crashes at module-load (ERR_MODULE_NOT_FOUND). The generic
# ${AUTOSPEC_SCRIPTS_DIR} guard above can't see static ES imports, so check the
# import graph directly. This is the bug where doc-scaffold.mjs + doc-coverage.mjs
# were omitted from install.sh's autospec_doc_scripts closure list.

@test "autospec-doc orchestrator import closure is fully shipped by install.sh" {
  scripts_dir="$REPO_ROOT/skills/autospec-doc/scripts"
  seen=" "
  queue="doc-orchestrator.mjs"
  closure=""
  while [ -n "$queue" ]; do
    next=""
    for m in $queue; do
      case "$seen" in *" $m "*) continue ;; esac
      seen="$seen$m "
      closure="$closure $m"
      imports="$(grep -hoE "from '\./[a-z-]+\.mjs'" "$scripts_dir/$m" 2>/dev/null | sed "s#from '\./##;s#'##")"
      next="$next $imports"
    done
    queue="$next"
  done
  shipped="$(grep -oE 'skills/autospec-doc/scripts/[a-z-]+\.mjs' "$REPO_ROOT/install.sh" | sed 's#.*/##' | sort -u)"
  missing=""
  for m in $closure; do
    printf '%s\n' "$shipped" | grep -qx "$m" || missing="$missing $m"
  done
  [ -z "$missing" ] || { echo "Unshipped doc-orchestrator closure modules:$missing" >&2; false; }
}

@test "autospec-doc flat entry is the delegating shim (not the real orchestrator)" {
  # The flat ${AUTOSPEC_SCRIPTS_DIR}/doc-orchestrator.mjs must be the shim, since a
  # flat copy of the real orchestrator can't resolve gen-audience-docs' two-level
  # ../../autospec-shared/scripts import.
  run grep -qE 'skills/autospec-doc/scripts/doc-orchestrator-entry\.mjs::doc-orchestrator\.mjs' "$REPO_ROOT/install.sh"
  [ "$status" -eq 0 ]
  [ -f "$REPO_ROOT/skills/autospec-doc/scripts/doc-orchestrator-entry.mjs" ]
  # The shim builds the subtree path from segments and re-execs it.
  run grep -qE "spawnSync" "$REPO_ROOT/skills/autospec-doc/scripts/doc-orchestrator-entry.mjs"
  [ "$status" -eq 0 ]
  run grep -qE "autospec-doc" "$REPO_ROOT/skills/autospec-doc/scripts/doc-orchestrator-entry.mjs"
  [ "$status" -eq 0 ]
}
