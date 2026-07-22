#!/usr/bin/env bash
# lint-fab-dockerfile.sh — host-portable pin lint for the autospec-fab container
# Dockerfile (issue #1300; folded from the deferred #1301 nightly CI). Asserts the
# Dockerfile pins EVERY version so the multi-GB image is reproducible:
#   - FROM uses a @sha256 digest (never a floating tag, never :latest),
#   - no ":latest" anywhere,
#   - every `apt-get install` package carries an exact `=<version>` pin,
#   - pip installs use --require-hashes.
# Runs WITHOUT building the image (the build itself runs only in the deferred
# nightly fab-container.yml). Exit 0 = all pins present; non-zero = a floating pin.
#
# Usage: scripts/lint-fab-dockerfile.sh [path/to/Dockerfile]

set -euo pipefail

DOCKERFILE="${1:-skills/autospec-fab/docker/Dockerfile}"

err() { printf 'lint-fab-dockerfile: FAIL: %s\n' "$1" >&2; }

if [ ! -f "$DOCKERFILE" ]; then
    err "Dockerfile not found: $DOCKERFILE"
    exit 1
fi

rc=0

# --- 1. No floating :latest anywhere (comments excluded). ---
if grep -nE '^[^#]*:latest' "$DOCKERFILE" >/dev/null 2>&1; then
    err "floating ':latest' tag found:"
    grep -nE '^[^#]*:latest' "$DOCKERFILE" >&2
    rc=1
fi

# --- 2. Every FROM must pin by @sha256 digest. ---
# FROM lines may reference earlier build stages by name (FROM base AS …);
# those are local stages, not registry images — allow them. A registry image
# reference (contains a registry/repo path or a known image name) must carry
# @sha256. We enforce: each FROM whose image token is not a prior stage name and
# is not already a build-arg-driven digest must contain '@sha256:'.
stage_names=""
while IFS= read -r line; do
    case "$line" in
        \#*) continue ;;
    esac
    # Extract the image token (2nd field) and optional "AS <stage>".
    set -- $line
    [ "${1:-}" = "FROM" ] || continue
    image="${2:-}"
    # Record stage alias if present (… AS name).
    alias=""
    if [ "${3:-}" = "AS" ] || [ "${3:-}" = "as" ]; then
        alias="${4:-}"
    fi
    # If the image references a known prior stage, it's a local stage ref — OK.
    is_stage=0
    for s in $stage_names; do
        [ "$image" = "$s" ] && is_stage=1 && break
    done
    if [ "$is_stage" -eq 0 ]; then
        case "$image" in
            *@sha256:*) : ;;          # literal digest — OK
            *@\$\{*DIGEST*\}*) : ;;   # ARG-driven digest (e.g. @${UBUNTU_DIGEST}) — OK
            *@\$*DIGEST*) : ;;        # ARG-driven digest, unbraced — OK
            *)
                err "FROM not pinned by @sha256 digest: $line"
                rc=1
                ;;
        esac
    fi
    [ -n "$alias" ] && stage_names="$stage_names $alias"
done < "$DOCKERFILE"

# --- 2b. Each *_DIGEST ARG default must itself be a real sha256 digest, so an
#         ARG-driven FROM (@${UBUNTU_DIGEST}) can't smuggle a floating value. ---
while IFS= read -r dl; do
    case "$dl" in \#*) continue ;; esac
    val="${dl#*=}"
    case "$val" in
        sha256:[0-9a-f]*) : ;;
        *)
            err "ARG *_DIGEST default is not a sha256 digest: $dl"
            rc=1
            ;;
    esac
done < <(grep -E '^ARG +[A-Z_]*DIGEST=' "$DOCKERFILE" || true)

# --- 3. Every apt-get install package must carry an exact =<version> pin. ---
# Tokenize install lines (handling line continuations) and check each package
# token (not a flag, not apt-get/install) contains '='. ARG/ENV interpolation
# like pkg=${VERSION} counts as a pin.
apt_block=""
in_apt=0
while IFS= read -r raw; do
    line="$raw"
    case "$line" in
        \#*) continue ;;
    esac
    if printf '%s' "$line" | grep -qE 'apt-get +install'; then
        in_apt=1
        apt_block=""
    fi
    if [ "$in_apt" -eq 1 ]; then
        apt_block="$apt_block $line"
        # Continuation? keep accumulating; else close the block.
        case "$line" in
            *\\) : ;;
            *)
                in_apt=0
                # Strip everything up to and including 'install', plus an && tail.
                pkgs="${apt_block#*install}"
                pkgs="${pkgs%%&&*}"
                for tok in $pkgs; do
                    case "$tok" in
                        -*|\\|apt-get|install) continue ;;
                        *=*) continue ;;            # pinned (pkg=ver or pkg=${VER})
                        *)
                            err "unpinned apt package (no =version): '$tok' in: $(printf '%s' "$apt_block" | tr -s ' ')"
                            rc=1
                            ;;
                    esac
                done
                ;;
        esac
    fi
done < "$DOCKERFILE"

# --- 4. pip install must use --require-hashes. ---
if grep -nE 'pip +install' "$DOCKERFILE" >/dev/null 2>&1; then
    while IFS= read -r pl; do
        case "$pl" in \#*) continue ;; esac
        printf '%s' "$pl" | grep -q -- '--require-hashes' || {
            err "pip install without --require-hashes: $pl"
            rc=1
        }
    done < <(grep -nE 'pip +install' "$DOCKERFILE")
fi

if [ "$rc" -eq 0 ]; then
    printf 'lint-fab-dockerfile: OK — %s fully pinned (digest base, =version apt, hashed pip, no :latest)\n' "$DOCKERFILE"
fi
exit "$rc"
