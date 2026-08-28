#!/usr/bin/env bash
# scripts/autospec-control-plane.sh — local control-plane bootstrap helpers.

set -eu

usage() {
    cat <<'USAGE'
Usage:
  scripts/autospec-control-plane.sh --help
  scripts/autospec-control-plane.sh bootstrap --dry-run [--owner OWNER] [--governance-repo NAME] [--observatory-repo NAME]
  scripts/autospec-control-plane.sh bootstrap --confirm --owner OWNER --governance-repo NAME --observatory-repo NAME

Commands:
  bootstrap --dry-run    Print governance and observatory scaffolds without GitHub writes.
  bootstrap --confirm    Create/adopt companion repos, commit scaffolds, push, and write .autospec/control-plane.json.

Defaults:
  --owner OWNER             berlinguyinca (dry-run only; --confirm requires explicit value)
  --governance-repo NAME    autospec-governance (dry-run only; --confirm requires explicit value)
  --observatory-repo NAME   autospec-observatory (dry-run only; --confirm requires explicit value)

Environment:
  AUTOSPEC_OBSERVATORY_URL         Enables bootstrap event emission via scripts/autospec-observatory-events.sh
The dry-run renderer is intentionally offline-only: it prints policy files,
rules, schemas, fixtures, tests, docs, and observatory service scaffold files
planned for companion repositories and never creates repositories, commits,
pushes, or invokes gh.
USAGE
}

fail() {
    printf 'autospec-control-plane: %s\n' "$*" >&2
    exit 2
}

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CONTROL_PLANE_RENDER_LIB="${CONTROL_PLANE_RENDER_LIB:-$(cd "$(dirname "$0")" && pwd)/lib/autospec-control-plane-render.sh}"
# shellcheck source=scripts/lib/autospec-control-plane-render.sh
. "$CONTROL_PLANE_RENDER_LIB"
CONTROL_PLANE_OBSERVATORY_RENDER_LIB="${CONTROL_PLANE_OBSERVATORY_RENDER_LIB:-$(cd "$(dirname "$0")" && pwd)/lib/autospec-control-plane-observatory-render.sh}"
# shellcheck source=scripts/lib/autospec-control-plane-observatory-render.sh
. "$CONTROL_PLANE_OBSERVATORY_RENDER_LIB"

now_iso() { date -u +'%Y-%m-%dT%H:%M:%SZ'; }

require_command() {
    command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}


repo_full_name() {
    printf '%s/%s' "$1" "$2"
}

validate_owner_and_repo_names() {
    owner="$1"
    governance_repo="$2"
    observatory_repo="$3"
    printf '%s' "$owner" | grep -Eq '^[A-Za-z0-9][A-Za-z0-9-]{0,38}$'         || fail "--owner must be a GitHub owner slug containing only letters, numbers, or hyphens"
    for repo_name in "$governance_repo" "$observatory_repo"; do
        printf '%s' "$repo_name" | grep -Eq '^[A-Za-z0-9._-]+$'             || fail "repo names must contain only letters, numbers, dots, underscores, or hyphens"
        case "$repo_name" in
            .*|*/*|*'..'*) fail "repo names must not be hidden, contain slashes, or contain '..'" ;;
        esac
    done
}

repo_url_for() {
    full_name="$1"
    gh repo view "$full_name" --json url,defaultBranchRef --jq .url 2>/dev/null || return 1
}

create_repo_if_missing() {
    full_name="$1"
    description="$2"
    if repo_url_for "$full_name" >/dev/null 2>&1; then
        printf 'adopted'
        return 0
    fi
    gh repo create "$full_name" --public --description "$description" >/dev/null
    printf 'created'
}

write_rendered_files() {
    repo_name="$1"
    target_dir="$2"
    rendered_file="$3"
    current_file=""

    while IFS= read -r line || [ -n "$line" ]; do
        case "$line" in
            "--- ${repo_name}/"*" ---")
                rel_path="${line#--- ${repo_name}/}"
                rel_path="${rel_path% ---}"
                current_file="$target_dir/$rel_path"
                mkdir -p "$(dirname "$current_file")"
                : > "$current_file"
                ;;
            *)
                if [ -n "$current_file" ]; then
                    printf '%s\n' "$line" >> "$current_file"
                fi
                ;;
        esac
    done < "$rendered_file"
}

write_governance_extra_files() {
    target_dir="$1"
    mkdir -p "$target_dir/tests" "$target_dir/docs"
    cat > "$target_dir/tests/policy-schema.bats" <<'EOF'
#!/usr/bin/env bash
if [ -z "${BATS_VERSION:-}" ]; then exec bats "$0" "$@"; fi

@test "policy packs declare required governance fields" {
  grep -R "^policy_id:" policies >/dev/null
  grep -R "^privacy_tier:" policies >/dev/null
}
EOF
    for test_name in priority-resolution privacy-tier merge-rules project-classification cost-limits evidence-requirements; do
        cat > "$target_dir/tests/${test_name}.bats" <<EOF
#!/usr/bin/env bash
if [ -z "\${BATS_VERSION:-}" ]; then exec bats "\$0" "\$@"; fi

@test "${test_name} fixture exists" {
  [ -d policies ]
  [ -d rules ]
  [ -d fixtures/projects ]
}
EOF
    done
    cat > "$target_dir/docs/policy-authoring.md" <<'EOF'
# Policy Authoring

Policies are versioned data files validated by schema plus fixture tests.
EOF
    cat > "$target_dir/docs/project-classes.md" <<'EOF'
# Project Classes

Project classes map repositories to default policy packs and privacy tiers.
EOF
    cat > "$target_dir/docs/priority-waterfall.md" <<'EOF'
# Priority Waterfall

Priority waterfalls define the deterministic ordering autospec uses when selecting work.
EOF
}

scaffold_governance_repo() {
    repo_name="$1"
    target_dir="$2"
    rendered_file="$(mktemp)"
    render_governance_file_templates "$repo_name" > "$rendered_file"
    write_rendered_files "$repo_name" "$target_dir" "$rendered_file"
    rm -f "$rendered_file"
    write_governance_extra_files "$target_dir"
}

scaffold_observatory_repo() {
    repo_name="$1"
    target_dir="$2"
    rendered_file="$(mktemp)"
    render_observatory_file_templates "$repo_name" > "$rendered_file"
    write_rendered_files "$repo_name" "$target_dir" "$rendered_file"
    rm -f "$rendered_file"
}

ensure_git_identity() {
    repo_dir="$1"
    git -C "$repo_dir" config user.name >/dev/null 2>&1 || git -C "$repo_dir" config user.name "autospec-control-plane"
    git -C "$repo_dir" config user.email >/dev/null 2>&1 || git -C "$repo_dir" config user.email "autospec-control-plane@example.invalid"
}

commit_and_push_if_changed() {
    repo_dir="$1"
    commit_message="$2"
    ensure_git_identity "$repo_dir"
    git -C "$repo_dir" add -A
    if [ -n "$(git -C "$repo_dir" status --porcelain)" ]; then
        git -C "$repo_dir" commit -m "$commit_message" >/dev/null
    fi
    current_branch="$(git -C "$repo_dir" symbolic-ref --quiet --short HEAD 2>/dev/null || true)"
    if [ -z "$current_branch" ]; then
        current_branch="main"
        git -C "$repo_dir" checkout -B "$current_branch" >/dev/null
    fi
    git -C "$repo_dir" push -u origin "$current_branch" >/dev/null
}

clone_or_init_repo() {
    repo_url="$1"
    clone_dir="$2"
    if git clone "$repo_url" "$clone_dir" >/dev/null 2>&1; then
        if ! git -C "$clone_dir" symbolic-ref --quiet --short HEAD >/dev/null 2>&1; then
            git -C "$clone_dir" checkout -B main >/dev/null
        fi
        return 0
    fi
    mkdir -p "$clone_dir"
    git -C "$clone_dir" init >/dev/null
    git -C "$clone_dir" checkout -B main >/dev/null
    git -C "$clone_dir" remote add origin "$repo_url"
}

ensure_companion_repo() {
    owner="$1"
    repo_name="$2"
    kind="$3"
    work_root="$4"
    full_name="$(repo_full_name "$owner" "$repo_name")"

    state="$(create_repo_if_missing "$full_name" "Autospec ${kind} companion repository")"
    repo_url="$(repo_url_for "$full_name")"
    clone_dir="$work_root/$repo_name"
    rm -rf "$clone_dir"
    clone_or_init_repo "$repo_url" "$clone_dir"

    case "$kind" in
        governance) scaffold_governance_repo "$repo_name" "$clone_dir" ;;
        observatory) scaffold_observatory_repo "$repo_name" "$clone_dir" ;;
        *) fail "unknown companion repo kind: $kind" ;;
    esac

    commit_and_push_if_changed "$clone_dir" "feat: scaffold autospec $kind repo"
    printf '%s\t%s\t%s\n' "$state" "$repo_url" "$clone_dir"
}

register_companion_repo() {
    full_name="$1"
    state="$2"
    autospec_bin="${AUTOSPEC_BIN:-autospec}"
    if [ "$state" = "created" ]; then
        spawned_from="${AUTOSPEC_SOURCE_SPEC:-${AUTOSPEC_RUN_ID:-control-plane-bootstrap}}"
        if ! "$autospec_bin" project onboard --repo-dir "$PWD" --repo "$full_name" \
          --spawned-from "$spawned_from"; then
            printf '%s\n' 'WARNING: managed Project repository registration failed; projection remains pending' >&2
        fi
    elif ! "$autospec_bin" project onboard --repo-dir "$PWD" --repo "$full_name"; then
        printf '%s\n' 'WARNING: managed Project repository registration failed; projection remains pending' >&2
    fi
}

emit_bootstrap_event() {
    event_type="$1"
    summary="$2"
    repository_id="$3"
    [ -n "${AUTOSPEC_OBSERVATORY_URL:-}" ] || return 0
    events_script="$SCRIPT_DIR/autospec-observatory-events.sh"
    [ -x "$events_script" ] || [ -f "$events_script" ] || return 0
    AUTOSPEC_RUN_ID="${AUTOSPEC_RUN_ID:-control-plane-bootstrap}" \
      bash "$events_script" emit \
        --run-id "${AUTOSPEC_RUN_ID:-control-plane-bootstrap}" \
        --event-type "$event_type" \
        --repository-id "$repository_id" \
        --status completed \
        --summary "$summary" >/dev/null
}

write_control_plane_config() {
    owner="$1"
    governance_repo="$2"
    governance_url="$3"
    governance_state="$4"
    observatory_repo="$5"
    observatory_url="$6"
    observatory_state="$7"
    config_path=".autospec/control-plane.json"
    mkdir -p .autospec
    completed_at="$(now_iso)"
    tmp="$(mktemp .autospec/control-plane.XXXXXX)"
    jq -n \
      --arg owner "$owner" \
      --arg governance_repo "$governance_repo" \
      --arg governance_url "$governance_url" \
      --arg governance_state "$governance_state" \
      --arg observatory_repo "$observatory_repo" \
      --arg observatory_url "$observatory_url" \
      --arg observatory_state "$observatory_state" \
      --arg completed_at "$completed_at" \
      --arg observatory_endpoint "${AUTOSPEC_OBSERVATORY_URL:-}" \
      '{
        owner: $owner,
        governance: {repo: $governance_repo, full_name: ($owner + "/" + $governance_repo), url: $governance_url, bootstrap_state: $governance_state},
        observatory: {repo: $observatory_repo, full_name: ($owner + "/" + $observatory_repo), url: $observatory_url, bootstrap_state: $observatory_state, endpoint: (if $observatory_endpoint == "" then null else $observatory_endpoint end)},
        bootstrap: {confirmed: true, completed_at: $completed_at, tool: "scripts/autospec-control-plane.sh bootstrap --confirm"}
      }' > "$tmp"
    mv "$tmp" "$config_path"
}

bootstrap_confirm() {
    owner="$1"
    governance_repo="$2"
    observatory_repo="$3"

    require_command gh
    require_command git
    require_command jq

    work_root="${AUTOSPEC_CONTROL_PLANE_WORKDIR:-}"
    if [ -z "$work_root" ]; then
        work_root="$(mktemp -d)"
    else
        mkdir -p "$work_root"
    fi

    emit_bootstrap_event "ControlPlaneBootstrapStarted" "control-plane bootstrap started" "$owner/autospec"

    governance_result="$(ensure_companion_repo "$owner" "$governance_repo" governance "$work_root")"
    governance_state="$(printf '%s' "$governance_result" | awk -F '\t' '{print $1}')"
    governance_url="$(printf '%s' "$governance_result" | awk -F '\t' '{print $2}')"
    register_companion_repo "$owner/$governance_repo" "$governance_state"
    emit_bootstrap_event "GovernanceRepoCreated" "governance repo ${governance_state}: ${governance_repo}" "$owner/$governance_repo"

    observatory_result="$(ensure_companion_repo "$owner" "$observatory_repo" observatory "$work_root")"
    observatory_state="$(printf '%s' "$observatory_result" | awk -F '\t' '{print $1}')"
    observatory_url="$(printf '%s' "$observatory_result" | awk -F '\t' '{print $2}')"
    register_companion_repo "$owner/$observatory_repo" "$observatory_state"
    emit_bootstrap_event "ObservatoryRepoCreated" "observatory repo ${observatory_state}: ${observatory_repo}" "$owner/$observatory_repo"

    write_control_plane_config "$owner" "$governance_repo" "$governance_url" "$governance_state" \
      "$observatory_repo" "$observatory_url" "$observatory_state"
    emit_bootstrap_event "ControlPlaneBootstrapCompleted" "control-plane bootstrap completed" "$owner/autospec"

    printf 'Control plane bootstrap completed\n'
    printf 'governance_repo=%s/%s\n' "$owner" "$governance_repo"
    printf 'governance_url=%s\n' "$governance_url"
    printf 'observatory_repo=%s/%s\n' "$owner" "$observatory_repo"
    printf 'observatory_url=%s\n' "$observatory_url"
    printf 'config=.autospec/control-plane.json\n'
}

bootstrap() {
    dry_run=0
    confirm=0
    owner="berlinguyinca"
    governance_repo="autospec-governance"
    observatory_repo="autospec-observatory"
    owner_explicit=0
    governance_explicit=0
    observatory_explicit=0

    while [ "$#" -gt 0 ]; do
        case "$1" in
            --dry-run)
                dry_run=1
                shift
                ;;
            --confirm)
                confirm=1
                shift
                ;;
            --owner)
                [ "$#" -ge 2 ] || fail "--owner requires a value"
                owner="$2"
                owner_explicit=1
                shift 2
                ;;
            --governance-repo)
                [ "$#" -ge 2 ] || fail "--governance-repo requires a value"
                governance_repo="$2"
                governance_explicit=1
                shift 2
                ;;
            --observatory-repo)
                [ "$#" -ge 2 ] || fail "--observatory-repo requires a value"
                observatory_repo="$2"
                observatory_explicit=1
                shift 2
                ;;
            --help|-h)
                usage
                exit 0
                ;;
            *)
                fail "unknown bootstrap argument: $1"
                ;;
        esac
    done

    [ "$dry_run" -eq 0 ] || [ "$confirm" -eq 0 ] || fail "bootstrap accepts only one of --dry-run or --confirm"
    if [ "$confirm" -eq 1 ]; then
        [ "$owner_explicit" -eq 1 ] && [ "$governance_explicit" -eq 1 ] && [ "$observatory_explicit" -eq 1 ] \
            || fail "--confirm requires --owner, --governance-repo, and --observatory-repo"
        [ -n "$owner" ] && [ -n "$governance_repo" ] && [ -n "$observatory_repo" ] \
            || fail "--confirm requires non-empty owner and repo names"
        validate_owner_and_repo_names "$owner" "$governance_repo" "$observatory_repo"
        bootstrap_confirm "$owner" "$governance_repo" "$observatory_repo"
        return 0
    fi

    [ "$dry_run" -eq 1 ] || fail "bootstrap requires --dry-run or --confirm"
    render_control_plane_dry_run "$owner" "$governance_repo" "$observatory_repo"
}

main() {
    if [ "$#" -eq 0 ]; then
        usage
        exit 0
    fi

    case "$1" in
        --help|-h)
            usage
            ;;
        bootstrap)
            shift
            bootstrap "$@"
            ;;
        *)
            fail "unknown command: $1"
            ;;
    esac
}

main "$@"
