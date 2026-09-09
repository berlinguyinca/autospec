#!/usr/bin/env bash
# scripts/lint-generated-artifacts.sh — generated-artifact consumer-drift ratchet.
#
# A generated artefact with two consumers needs two checks: regenerating it
# satisfies one (the generator's own --check) and silently breaks the other
# (a consumer that recorded a reference nobody re-verified). This ratchet
# covers every committed generated artefact under templates/generated/ and
# docs/generated/ (issue #3893):
#
#   ORPHAN_ARTIFACT   no generator (a *.sh at the repo root or under scripts/
#                     that mentions the artefact's repo-relative path) can be
#                     found, so nothing surfaces when its consumers change.
#   UNLISTED_ARTIFACT no generator enumerates the artefact in a
#                     `# Consumers(<artefact>): <paths...>` block. The
#                     enumeration lives in the generator so a change to the
#                     generator surfaces the consumer list.
#   STALE_REFERENCE   a recorded sha256/sha512 digest that mentions the
#                     artefact no longer matches the artefact's bytes. A
#                     recorded reference is a line that mentions the artefact
#                     (repo-relative path or basename) and carries a
#                     `sha256[: =_-]*<64 hex>` / `sha512[: =_-]*<128 hex>`
#                     digest, or a sha256sum-format `<64 hex>  <path>` line.
#   STALE_CONSUMER    an enumerated consumer no longer exists or no longer
#                     references the artefact (by path, or by directory+stem
#                     when the reference is parameterized, e.g.
#                     harness-runtime-aliases.$format).
#
# Usage:
#   scripts/lint-generated-artifacts.sh [--root DIR] [--list] [--help]
#
# Findings print to stdout, one per line:
#   <RULE>:<path>[:<line>]: <description>
# Exit 0 when clean; exit 1 on any finding. --list prints one INFO line per
# artefact and always exits 0 (audit mode).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LIST_ONLY=0

usage() {
    printf 'Usage: %s [--root DIR] [--list] [--help]\n' "$0"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --root) ROOT="${2:-}"; shift 2 ;;
        --list) LIST_ONLY=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; exit 2 ;;
    esac
done

ROOT="$(cd "$ROOT" && pwd -P)"

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        openssl dgst -sha256 "$1" | awk '{print $NF}'
    fi
}

sha512_of() {
    if command -v sha512sum >/dev/null 2>&1; then
        sha512sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 512 "$1" | awk '{print $1}'
    else
        openssl dgst -sha512 "$1" | awk '{print $NF}'
    fi
}

FINDINGS=""
LIST_LINES=""
add_finding() {
    if [ -z "$FINDINGS" ]; then
        FINDINGS="$1"
    else
        FINDINGS="${FINDINGS}"$'\n'"$1"
    fi
}
add_list_line() {
    if [ -z "$LIST_LINES" ]; then
        LIST_LINES="$1"
    else
        LIST_LINES="${LIST_LINES}"$'\n'"$1"
    fi
}

# check_line_digests FILE LINE HIT_LINE REL GEN_REL ACTUAL_256 ACTUAL_512
# One line of a file that mentions a generated artefact: every sha256/sha512
# digest it carries must match the artefact's bytes.
check_line_digests() {
    local FILE="$1" LINE="$2" HIT_LINE="$3" REL="$4" GEN_REL="$5" A256="$6" A512="$7"
    local TOK HASH
    while IFS= read -r TOK; do
        [ -n "$TOK" ] || continue
        HASH="$(printf '%s\n' "$TOK" | sed -E 's/^[Ss][Hh][Aa]256[[:space:]:=_-]*//')"
        if [ "${#HASH}" -eq 64 ] && [ "$(printf '%s' "$HASH" | tr 'A-F' 'a-f')" != "$A256" ]; then
            add_finding "STALE_REFERENCE:${FILE#\"$ROOT\"/}:${HIT_LINE}: recorded sha256 of ${REL} does not match the artefact (${HASH} != ${A256}) [generator: ${GEN_REL}]."
        fi
    done < <(printf '%s\n' "$LINE" | grep -oiE 'sha256[[:space:]:=_-]*[0-9a-fA-F]+' || true)
    while IFS= read -r TOK; do
        [ -n "$TOK" ] || continue
        HASH="$(printf '%s\n' "$TOK" | sed -E 's/^[Ss][Hh][Aa]512[[:space:]:=_-]*//')"
        if [ "${#HASH}" -eq 128 ] && [ "$(printf '%s' "$HASH" | tr 'A-F' 'a-f')" != "$A512" ]; then
            add_finding "STALE_REFERENCE:${FILE#\"$ROOT\"/}:${HIT_LINE}: recorded sha512 of ${REL} does not match the artefact (${HASH} != ${A512}) [generator: ${GEN_REL}]."
        fi
    done < <(printf '%s\n' "$LINE" | grep -oiE 'sha512[[:space:]:=_-]*[0-9a-fA-F]+' || true)
    # sha256sum manifest form: "<64 hex>  <path...>"
    if printf '%s\n' "$LINE" | grep -qE '^[0-9a-fA-F]{64}[[:space:]]' && printf '%s\n' "$LINE" | grep -qF "$REL"; then
        HASH="$(printf '%s' "$LINE" | cut -c1-64 | tr 'A-F' 'a-f')"
        if [ "$HASH" != "$A256" ]; then
            add_finding "STALE_REFERENCE:${FILE#\"$ROOT\"/}:${HIT_LINE}: recorded sha256 (sha256sum form) of ${REL} does not match the artefact (${HASH} != ${A256}) [generator: ${GEN_REL}]."
        fi
    fi
    return 0
}

# check_consumers REL REFPREFIX CONSUMERS GEN_REL
# Every enumerated consumer of a generated artefact must still exist and must
# still reference it (by full path, or by directory+stem when the reference
# is parameterized, e.g. harness-runtime-aliases.$format).
check_consumers() {
    local REL="$1" REFPREFIX="$2" CONSUMERS="$3" GEN_REL="$4"
    local CONS CONS_FILE
    for CONS in $CONSUMERS; do
        CONS_FILE="$ROOT/$CONS"
        if [ ! -f "$CONS_FILE" ]; then
            add_finding "STALE_CONSUMER:${CONS}: consumer of ${REL} no longer exists [generator: ${GEN_REL}]."
        elif ! grep -qF "$REL" -- "$CONS_FILE" && ! grep -qF "$REFPREFIX" -- "$CONS_FILE"; then
            add_finding "STALE_CONSUMER:${CONS}: consumer of ${REL} no longer references the artefact [generator: ${GEN_REL}]."
        fi
    done
    return 0
}

# Artefacts: every committed file under the two generated directories.
ARTIFACTS="$(
    {
        if [ -d "$ROOT/templates/generated" ]; then find "$ROOT/templates/generated" -type f; fi
        if [ -d "$ROOT/docs/generated" ]; then find "$ROOT/docs/generated" -type f; fi
        true
    } 2>/dev/null | LC_ALL=C sort
)"

if [ -n "$ARTIFACTS" ]; then
    while IFS= read -r ART; do
        [ -n "$ART" ] || continue
        REL="${ART#"$ROOT"/}"
        BASE="${REL##*/}"
        STEM="${BASE%.*}"
        if [ "$STEM" = "$BASE" ]; then
            REFPREFIX="$REL"
        else
            REFPREFIX="$(dirname "$REL")/${STEM}"
        fi
        REL_RE="$(printf '%s' "$REL" | sed 's/[.[\*^$]/\\&/g')"
        GEN=""
        GEN_ENUM=""
        # Generator candidates: *.sh at the root or under scripts/ that
        # mention the artefact's full repo-relative path. The artefact is
        # enumerated if any candidate carries its Consumers block.
        while IFS= read -r CAND; do
            [ -n "$CAND" ] || continue
            grep -qF "$REL" -- "$CAND" 2>/dev/null || continue
            if [ -z "$GEN" ]; then
                GEN="$CAND"
            fi
            ENUM_LINE="$(grep -E "^#[[:space:]]*Consumers\($REL_RE\):[[:space:]]*[^[:space:]]" -- "$CAND" 2>/dev/null | head -n 1 || true)"
            if [ -n "$ENUM_LINE" ] && [ -z "$GEN_ENUM" ]; then
                GEN_ENUM="$ENUM_LINE"
                GEN="$CAND"
            fi
        done < <(
            {
                find "$ROOT" -maxdepth 1 -type f -name '*.sh' 2>/dev/null
                if [ -d "$ROOT/scripts" ]; then find "$ROOT/scripts" -type f -name '*.sh' 2>/dev/null; fi
                true
            } | LC_ALL=C sort
        )
        if [ -z "$GEN" ]; then
            add_finding "ORPHAN_ARTIFACT:${REL}: no generator found (no *.sh at the repo root or under scripts/ mentions this path); an artefact with no generator has no check that surfaces consumer changes."
            continue
        fi
        GEN_REL="${GEN#"$ROOT"/}"
        if [ -z "$GEN_ENUM" ]; then
            CONSUMERS=""
            add_finding "UNLISTED_ARTIFACT:${REL}: not enumerated in ${GEN_REL}; add a '# Consumers(${REL}): <consumer paths...>' block to the generator so its consumer list surfaces on generator changes."
        else
            CONSUMERS="$(printf '%s\n' "$GEN_ENUM" | sed -E "s|^#[[:space:]]*Consumers\($REL_RE\):[[:space:]]*||")"
        fi
        if [ -n "$CONSUMERS" ]; then
            check_consumers "$REL" "$REFPREFIX" "$CONSUMERS" "$GEN_REL"
        fi
        # Recorded references: any text file line that mentions the artefact
        # and carries a sha256/sha512 digest must match the artefact's bytes.
        ACTUAL_256="$(sha256_of "$ART" | tr 'A-F' 'a-f')"
        ACTUAL_512="$(sha512_of "$ART" | tr 'A-F' 'a-f')"
        while IFS= read -r FILE; do
            [ -n "$FILE" ] || continue
            HITS="$(grep -In -e "$REL" -e "$BASE" -- "$FILE" 2>/dev/null || true)"
            [ -n "$HITS" ] || continue
            while IFS= read -r HIT; do
                [ -n "$HIT" ] || continue
                HIT_LINE="${HIT%%:*}"
                LINE="${HIT#*:}"
                check_line_digests "$FILE" "$LINE" "$HIT_LINE" "$REL" "$GEN_REL" "$ACTUAL_256" "$ACTUAL_512"
            done < <(printf '%s\n' "$HITS")
        done < <(
            find "$ROOT" \
                \( -name .git -o -name node_modules -o -name target -o -name dist -o -name vendor -o -name .autospec \) -prune \
                -o -type f -print 2>/dev/null | LC_ALL=C sort
        )
        if [ "$LIST_ONLY" -eq 1 ]; then
            if [ -z "$GEN_ENUM" ]; then
                add_list_line "INFO:${REL}: generator=${GEN_REL} consumers=none (UNLISTED)"
            else
                add_list_line "INFO:${REL}: generator=${GEN_REL} consumers=$CONSUMERS"
            fi
        fi
    done < <(printf '%s\n' "$ARTIFACTS")
fi

if [ "$LIST_ONLY" -eq 1 ]; then
    if [ -n "$LIST_LINES" ]; then
        printf '%s\n' "$LIST_LINES"
    fi
    exit 0
fi

if [ -n "$FINDINGS" ]; then
    printf '%s\n' "$FINDINGS"
    exit 1
fi
exit 0
