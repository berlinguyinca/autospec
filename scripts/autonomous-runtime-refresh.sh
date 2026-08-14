#!/usr/bin/env bash
# Runtime source identity and immutable-generation freshness checks.

autospec_runtime_receipt_schema=4

autospec_runtime_error() {
    printf 'error:%s\n' "$1" >&2
    return 2
}

autospec_runtime_valid_sha256() {
    local LC_ALL=C
    [[ ${1-} =~ ^[0-9a-f]{64}$ ]]
}

autospec_runtime_sha256_file() {
    local raw digest
    if command -v sha256sum >/dev/null 2>&1; then
        raw=$(sha256sum "$1" 2>/dev/null) || { autospec_runtime_error sha256-failed; return 2; }
    elif command -v shasum >/dev/null 2>&1; then
        raw=$(shasum -a 256 "$1" 2>/dev/null) || { autospec_runtime_error sha256-failed; return 2; }
    else
        autospec_runtime_error sha256-unavailable
        return 2
    fi
    digest=${raw%%[[:space:]]*}
    autospec_runtime_valid_sha256 "$digest" || { autospec_runtime_error sha256-invalid; return 2; }
    printf '%s\n' "$digest"
}

autospec_runtime_file_sha256() {
    [ -f "$1" ] && [ ! -L "$1" ] && [ -r "$1" ] || {
        autospec_runtime_error file-unreadable
        return 2
    }
    autospec_runtime_sha256_file "$1"
}

autospec_runtime_repo_dir() {
    local repo
    [ -d "${1-}" ] || { autospec_runtime_error repo-dir-invalid; return 2; }
    repo=$(git -C "$1" rev-parse --show-toplevel 2>/dev/null) || {
        autospec_runtime_error repo-dir-invalid
        return 2
    }
    (CDPATH='' cd -P -- "$repo" 2>/dev/null && pwd -P) || {
        autospec_runtime_error repo-dir-invalid
        return 2
    }
}

autospec_runtime_path_relevant() {
    case "$1" in
        target/*|*/target/*) return 1 ;;
        *) return 0 ;;
    esac
}

autospec_runtime_path_safe() {
    local LC_ALL=C
    case "$1" in ''|*[!A-Za-z0-9._/@+=,\ -]*) return 1 ;; *) return 0 ;; esac
}

autospec_runtime_temp_file() {
    mktemp "${TMPDIR:-/tmp}/autospec-runtime.XXXXXX" || {
        autospec_runtime_error temporary-file-unavailable
        return 2
    }
}

autospec_runtime_source_digest() (
    local repo raw paths sorted stream path size
    raw='' paths='' sorted='' stream=''
    # Invoked by the EXIT trap.
    # shellcheck disable=SC2329
    cleanup() { rm -f "$raw" "$paths" "$sorted" "$stream"; }
    trap cleanup EXIT
    trap 'exit 2' HUP INT TERM
    LC_ALL=C
    export LC_ALL
    repo=$(autospec_runtime_repo_dir "${1-}") || exit 2
    raw=$(autospec_runtime_temp_file) || exit 2
    paths=$(autospec_runtime_temp_file) || exit 2
    sorted=$(autospec_runtime_temp_file) || exit 2
    stream=$(autospec_runtime_temp_file) || exit 2
    git -C "$repo" ls-files -z -co --exclude-standard >"$raw" || {
        autospec_runtime_error source-list-failed
        exit 2
    }
    : >"$paths"
    while IFS= read -r -d '' path; do
        autospec_runtime_path_relevant "$path" || continue
        autospec_runtime_path_safe "$path" || { autospec_runtime_error source-path-unsafe; exit 2; }
        printf '%s\n' "$path" >>"$paths" || exit 2
    done <"$raw"
    sort "$paths" >"$sorted" || { autospec_runtime_error source-sort-failed; exit 2; }
    : >"$stream"
    while IFS= read -r path; do
        [ -n "$path" ] || continue
        if [ ! -e "$repo/$path" ]; then
            printf 'D%s\0%s\0' "${#path}" "$path" >>"$stream" || exit 2
            continue
        fi
        [ -f "$repo/$path" ] && [ ! -L "$repo/$path" ] && [ -r "$repo/$path" ] || {
            autospec_runtime_error source-input-invalid
            exit 2
        }
        size=$(wc -c <"$repo/$path") || { autospec_runtime_error source-read-failed; exit 2; }
        size=${size//[[:space:]]/}
        printf 'F%s\0%s\0%s\0' "${#path}" "$path" "$size" >>"$stream" || exit 2
        cat "$repo/$path" >>"$stream" || { autospec_runtime_error source-read-failed; exit 2; }
    done <"$sorted"
    autospec_runtime_sha256_file "$stream"
)

autospec_runtime_head() {
    local head
    head=$(git -C "$1" rev-parse --verify HEAD 2>/dev/null) || { autospec_runtime_error repo-head-unavailable; return 2; }
    [[ $head =~ ^([0-9a-f]{40}|[0-9a-f]{64})$ ]] || { autospec_runtime_error repo-head-invalid; return 2; }
    printf '%s\n' "$head"
}

autospec_runtime_identity_tuple() {
    local repo head source identity
    repo=$(autospec_runtime_repo_dir "$1") || return 2
    head=$(autospec_runtime_head "$repo") || return 2
    source=$(autospec_runtime_source_digest "$repo") || return 2
    identity=$(autospec_runtime_tuple_digest "$repo" "$head" "$source") || return 2
    printf '%s\n%s\n%s\n%s\n' "$repo" "$head" "$source" "$identity"
}

autospec_runtime_source_checkout() {
    local target state_root generations target_digest generation source
    target=$(autospec_runtime_repo_dir "${1-}") || return 2
    if [ -f "$target/Cargo.toml" ] && [ -f "$target/crates/autospec-cli/Cargo.toml" ]; then
        printf '%s\n' "$target"
        return 0
    fi

    state_root=${AUTOSPEC_STATE_ROOT:-$HOME/.autospec}
    generations=${AUTOSPEC_RUNTIME_ROOT:-$state_root/runtime-generations}
    autospec_runtime_private_dir "$state_root" || { autospec_runtime_error source-state-untrusted; return 2; }
    autospec_runtime_private_dir "$generations" || { autospec_runtime_error source-state-untrusted; return 2; }
    [ -L "$generations/current" ] || { autospec_runtime_error source-receipt-unavailable; return 2; }
    target_digest=$(readlink "$generations/current") || return 2
    autospec_runtime_valid_sha256 "$target_digest" || { autospec_runtime_error source-receipt-invalid; return 2; }
    [ "$target_digest" = "${target_digest##*/}" ] || { autospec_runtime_error source-receipt-invalid; return 2; }
    generation="$generations/$target_digest"
    autospec_runtime_parse_receipt "$generation/receipt" || { autospec_runtime_error source-receipt-invalid; return 2; }
    source=$(autospec_runtime_repo_dir "$receipt_repo") || { autospec_runtime_error source-checkout-unavailable; return 2; }
    [ -f "$source/Cargo.toml" ] && [ -f "$source/crates/autospec-cli/Cargo.toml" ] \
        || { autospec_runtime_error source-checkout-invalid; return 2; }
    autospec_runtime_verify_generation "$source" "$target_digest" "$generation" \
        || { autospec_runtime_error source-receipt-invalid; return 2; }
    printf '%s\n' "$source"
}

autospec_runtime_tuple_digest() {
    local repo=$1 head=$2 source=$3 tuple
    tuple=$(autospec_runtime_temp_file) || return 2
    trap 'rm -f "$tuple"' RETURN
    printf 'repo=%s\0head=%s\0source=%s\0' "$repo" "$head" "$source" >"$tuple" || return 2
    autospec_runtime_sha256_file "$tuple"
    rm -f "$tuple"
    trap - RETURN
}

autospec_runtime_stat_mode() {
    stat -f '%Lp' "$1" 2>/dev/null || stat -c '%a' "$1" 2>/dev/null
}

autospec_runtime_stat_owner() {
    stat -f '%u' "$1" 2>/dev/null || stat -c '%u' "$1" 2>/dev/null
}

autospec_runtime_private_dir() {
    local mode owner
    [ -d "$1" ] && [ ! -L "$1" ] || return 1
    mode=$(autospec_runtime_stat_mode "$1") || return 1
    owner=$(autospec_runtime_stat_owner "$1") || return 1
    [ "$mode" = 700 ] && [ "$owner" = "$(id -u)" ]
}

autospec_runtime_valid_timestamp() {
    local value year month day hour minute second max_day
    value=$1
    [[ $value =~ ^([0-9]{4})-([0-9]{2})-([0-9]{2})T([0-9]{2}):([0-9]{2}):([0-9]{2})Z$ ]] || return 1
    year=$((10#${BASH_REMATCH[1]})); month=$((10#${BASH_REMATCH[2]})); day=$((10#${BASH_REMATCH[3]}))
    hour=$((10#${BASH_REMATCH[4]})); minute=$((10#${BASH_REMATCH[5]})); second=$((10#${BASH_REMATCH[6]}))
    case "$month" in 1|3|5|7|8|10|12) max_day=31 ;; 4|6|9|11) max_day=30 ;;
        2) max_day=28; if [ $((year % 400)) -eq 0 ] || { [ $((year % 4)) -eq 0 ] && [ $((year % 100)) -ne 0 ]; }; then max_day=29; fi ;;
        *) return 1 ;;
    esac
    [ "$day" -ge 1 ] && [ "$day" -le "$max_day" ] && [ "$hour" -le 23 ] \
        && [ "$minute" -le 59 ] && [ "$second" -le 59 ]
}

autospec_runtime_write_receipt() {
    local repo binary output source_digest identity_digest binary_digest head clean_before clean_after snapshot installed_at temporary parent
    repo=$(autospec_runtime_repo_dir "$1") || return 2
    binary=$2; output=$3; source_digest=$4; identity_digest=$5; head=$6; clean_before=$7; clean_after=$8; snapshot=$9
    autospec_runtime_valid_sha256 "$source_digest" || return 2
    autospec_runtime_valid_sha256 "$identity_digest" || return 2
    binary_digest=$(autospec_runtime_file_sha256 "$binary") || return 2
    [[ $head =~ ^([0-9a-f]{40}|[0-9a-f]{64})$ ]] || { autospec_runtime_error repo-head-invalid; return 2; }
    case "$clean_before:$clean_after" in 0:0|0:1|1:0|1:1) ;; *) return 2 ;; esac
    autospec_runtime_valid_sha256 "$snapshot" || return 2
    case "$repo" in *$'\n'*|*$'\r'*) autospec_runtime_error receipt-repo-path-unsafe; return 2 ;; esac
    installed_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ') || return 2
    parent=$(dirname "$output")
    autospec_runtime_private_dir "$parent" || { autospec_runtime_error receipt-parent-untrusted; return 2; }
    if [ -L "$output" ] || { [ -e "$output" ] && [ ! -f "$output" ]; }; then
        autospec_runtime_error receipt-target-invalid
        return 2
    fi
    temporary=$(mktemp "$parent/.receipt.XXXXXX") || return 2
    trap 'rm -f "$temporary"' RETURN
    chmod 600 "$temporary" || return 2
    {
        printf 'schema=%s\n' "$autospec_runtime_receipt_schema"
        printf 'repo_dir=%s\n' "$repo"
        printf 'head=%s\n' "$head"
        printf 'source_sha256=%s\n' "$source_digest"
        printf 'identity_sha256=%s\n' "$identity_digest"
        printf 'clean_before=%s\n' "$clean_before"
        printf 'clean_after=%s\n' "$clean_after"
        printf 'snapshot_sha256=%s\n' "$snapshot"
        printf 'binary_sha256=%s\n' "$binary_digest"
        printf 'installed_at=%s\n' "$installed_at"
    } >"$temporary" || return 2
    mv "$temporary" "$output" || return 2
    trap - RETURN
}

autospec_runtime_parse_receipt() {
    local receipt line1 line2 line3 line4 line5 line6 line7 line8 line9 line10 extra mode owner
    receipt=$1
    [ -f "$receipt" ] && [ ! -L "$receipt" ] || return 1
    mode=$(autospec_runtime_stat_mode "$receipt") || return 1
    owner=$(autospec_runtime_stat_owner "$receipt") || return 1
    { [ "$mode" = 600 ] || [ "$mode" = 400 ]; } && [ "$owner" = "$(id -u)" ] || return 1
    exec 3<"$receipt" || return 1
    if ! IFS= read -r line1 <&3 || ! IFS= read -r line2 <&3 || ! IFS= read -r line3 <&3 \
        || ! IFS= read -r line4 <&3 || ! IFS= read -r line5 <&3 || ! IFS= read -r line6 <&3 \
        || ! IFS= read -r line7 <&3 || ! IFS= read -r line8 <&3 || ! IFS= read -r line9 <&3 \
        || ! IFS= read -r line10 <&3; then
        exec 3<&-
        return 1
    fi
    if IFS= read -r extra <&3 || [ -n "${extra-}" ]; then exec 3<&-; return 1; fi
    exec 3<&-
    case "$line1" in schema=*) receipt_schema=${line1#schema=} ;; *) return 1 ;; esac
    case "$line2" in repo_dir=*) receipt_repo=${line2#repo_dir=} ;; *) return 1 ;; esac
    case "$line3" in head=*) receipt_head=${line3#head=} ;; *) return 1 ;; esac
    case "$line4" in source_sha256=*) receipt_source=${line4#source_sha256=} ;; *) return 1 ;; esac
    case "$line5" in identity_sha256=*) receipt_identity=${line5#identity_sha256=} ;; *) return 1 ;; esac
    case "$line6" in clean_before=*) receipt_clean_before=${line6#clean_before=} ;; *) return 1 ;; esac
    case "$line7" in clean_after=*) receipt_clean_after=${line7#clean_after=} ;; *) return 1 ;; esac
    case "$line8" in snapshot_sha256=*) receipt_snapshot=${line8#snapshot_sha256=} ;; *) return 1 ;; esac
    case "$line9" in binary_sha256=*) receipt_binary=${line9#binary_sha256=} ;; *) return 1 ;; esac
    case "$line10" in installed_at=*) receipt_installed=${line10#installed_at=} ;; *) return 1 ;; esac
    case "$receipt_repo" in ''|*$'\n'*|*$'\r'*|*$'\t'*) return 1 ;; esac
    [ "$receipt_schema" = "$autospec_runtime_receipt_schema" ] \
        && autospec_runtime_valid_sha256 "$receipt_source" \
        && autospec_runtime_valid_sha256 "$receipt_identity" \
        && autospec_runtime_valid_sha256 "$receipt_binary" \
        && autospec_runtime_valid_sha256 "$receipt_snapshot" \
        && { [ "$receipt_clean_before" = 0 ] || [ "$receipt_clean_before" = 1 ]; } \
        && { [ "$receipt_clean_after" = 0 ] || [ "$receipt_clean_after" = 1 ]; } \
        && [[ $receipt_head =~ ^([0-9a-f]{40}|[0-9a-f]{64})$ ]] \
        && autospec_runtime_valid_timestamp "$receipt_installed"
}

autospec_runtime_verify_generation() {
    local repo digest generation binary receipt actual expected_identity repo_canonical current_head mode owner expected_mode
    repo=$1; digest=$2; generation=$3; binary="$generation/autospec"; receipt="$generation/receipt"
    expected_mode=${4:-500}
    repo_canonical=$(autospec_runtime_repo_dir "$repo") || return 2
    autospec_runtime_valid_sha256 "$digest" || return 1
    [ -d "$generation" ] && [ ! -L "$generation" ] || return 1
    mode=$(autospec_runtime_stat_mode "$generation") || return 1
    owner=$(autospec_runtime_stat_owner "$generation") || return 1
    [ "$mode" = "$expected_mode" ] && [ "$owner" = "$(id -u)" ] || return 1
    autospec_runtime_parse_receipt "$receipt" || return 1
    current_head=$(autospec_runtime_head "$repo_canonical") || return 1
    expected_identity=$(autospec_runtime_tuple_digest "$receipt_repo" "$receipt_head" "$receipt_source") || return 1
    [ "$receipt_repo" = "$repo_canonical" ] && [ "$receipt_head" = "$current_head" ] \
        && [ "$receipt_identity" = "$digest" ] && [ "$receipt_identity" = "$expected_identity" ] || return 1
    actual=$(autospec_runtime_file_sha256 "$binary") || return 1
    mode=$(autospec_runtime_stat_mode "$binary") || return 1
    owner=$(autospec_runtime_stat_owner "$binary") || return 1
    [ "$mode" = 500 ] && [ "$owner" = "$(id -u)" ] && [ "$receipt_binary" = "$actual" ]
}

autospec_runtime_verify_recorded_generation() {
    local generation=$1 digest=$2 repo=$3 head=$4 source=$5 expected_mode=${6:-500}
    local binary="$generation/autospec" receipt="$generation/receipt" expected_identity actual mode owner
    [ -d "$generation" ] && [ ! -L "$generation" ] || return 1
    mode=$(autospec_runtime_stat_mode "$generation") || return 1
    owner=$(autospec_runtime_stat_owner "$generation") || return 1
    [ "$mode" = "$expected_mode" ] && [ "$owner" = "$(id -u)" ] || return 1
    autospec_runtime_parse_receipt "$receipt" || return 1
    expected_identity=$(autospec_runtime_tuple_digest "$repo" "$head" "$source") || return 1
    [ "$receipt_repo" = "$repo" ] && [ "$receipt_head" = "$head" ] && [ "$receipt_source" = "$source" ] \
        && [ "$receipt_identity" = "$digest" ] && [ "$expected_identity" = "$digest" ] || return 1
    actual=$(autospec_runtime_file_sha256 "$binary") || return 1
    [ "$receipt_binary" = "$actual" ]
}

autospec_runtime_check() {
    exec bash "$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/autospec-runtime-install.sh" --check-only --repo-dir "$1"
}

autospec_runtime_usage() {
    printf 'usage: %s {source|identity|check|ensure} --repo-dir DIR\n' "${0##*/}" >&2
    return 2
}

autospec_runtime_main() {
    local action repo=''
    action=${1-}; [ -n "$action" ] || { autospec_runtime_usage; return 2; }; shift
    while [ "$#" -gt 0 ]; do
        case "$1" in --repo-dir) [ "$#" -ge 2 ] && [ -z "$repo" ] || { autospec_runtime_usage; return 2; }; repo=$2; shift 2 ;;
            *) autospec_runtime_usage; return 2 ;;
        esac
    done
    [ -n "$repo" ] || { autospec_runtime_usage; return 2; }
    case "$action" in
        source) autospec_runtime_source_checkout "$repo" ;;
        identity) autospec_runtime_identity_tuple "$repo" | sed -n '4p' ;;
        check) autospec_runtime_check "$repo" ;;
        ensure) exec bash "$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/autospec-runtime-install.sh" --repo-dir "$repo" ;;
        *) autospec_runtime_usage; return 2 ;;
    esac
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    autospec_runtime_main "$@"
fi
