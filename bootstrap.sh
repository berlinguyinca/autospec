#!/usr/bin/env bash
# bootstrap.sh — single-line curl-pipe installer for the autospec suite.
#
# Designed to be run via:
#   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash
#
# Or with forwarded flags:
#   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh \
#     | bash -s -- --skill autospec --harness claude
#
# What it does:
#   1. Pre-flights git + bash.
#   2. Clones (or fast-forwards) the repo to $AUTOSPEC_HOME/repo at $AUTOSPEC_REF.
#   3. cd into the checkout and runs ./install.sh, forwarding any extra args.
#   4. Exits with the installer's exit code.
#
# Honors:
#   AUTOSPEC_HOME       (default: $HOME/.autospec)  — base dir for repo + state
#   AUTOSPEC_REF        (default: main)             — git ref to install
#   AUTOSPEC_REPO_URL   (default: official remote)  — override for forks/mirrors
#   plus every variable install.sh already honors (AUTOSPEC_NO_STAR_PROMPT, etc.)

set -eu

AUTOSPEC_HOME="${AUTOSPEC_HOME:-$HOME/.autospec}"
AUTOSPEC_REF="${AUTOSPEC_REF:-main}"
AUTOSPEC_REPO_URL="${AUTOSPEC_REPO_URL:-https://github.com/berlinguyinca/autospec.git}"
REPO_DIR="$AUTOSPEC_HOME/repo"

err()  { printf 'bootstrap: error: %s\n' "$*" >&2; }
info() { printf 'bootstrap: %s\n' "$*"; }

require() {
    if ! command -v "$1" >/dev/null 2>&1; then
        err "$1 is required but not on PATH. Install it and re-run."
        exit 1
    fi
}

require git
require bash

mkdir -p "$AUTOSPEC_HOME"

if [ -d "$REPO_DIR/.git" ]; then
    info "updating existing checkout at $REPO_DIR (ref: $AUTOSPEC_REF)"
    git -C "$REPO_DIR" fetch --quiet origin "$AUTOSPEC_REF" || {
        err "git fetch failed for $AUTOSPEC_REPO_URL ($AUTOSPEC_REF)"
        exit 1
    }
    git -C "$REPO_DIR" checkout --quiet "$AUTOSPEC_REF"
    # Fast-forward only — preserve any uncommitted local edits with a clear error.
    if ! git -C "$REPO_DIR" merge --ff-only --quiet "origin/$AUTOSPEC_REF" 2>/dev/null; then
        # Detached HEAD on a tag/sha is fine; only warn if a fast-forward was expected.
        if git -C "$REPO_DIR" symbolic-ref --quiet HEAD >/dev/null 2>&1; then
            err "could not fast-forward $REPO_DIR to origin/$AUTOSPEC_REF. Resolve local changes and re-run."
            exit 1
        fi
    fi
elif [ -e "$REPO_DIR" ]; then
    err "$REPO_DIR exists but is not a git checkout. Remove it or set AUTOSPEC_HOME and re-run."
    exit 1
else
    info "cloning $AUTOSPEC_REPO_URL to $REPO_DIR (ref: $AUTOSPEC_REF)"
    git clone --quiet --branch "$AUTOSPEC_REF" --depth 1 "$AUTOSPEC_REPO_URL" "$REPO_DIR" 2>/dev/null \
        || git clone --quiet "$AUTOSPEC_REPO_URL" "$REPO_DIR"
    git -C "$REPO_DIR" checkout --quiet "$AUTOSPEC_REF"
fi

info "running $REPO_DIR/install.sh $*"
cd "$REPO_DIR"
exec bash ./install.sh "$@"
