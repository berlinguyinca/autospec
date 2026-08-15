#!/usr/bin/env bash
# Verifies offer_gitignore: dry-run announces, AUTOSPEC_AUTO_YES adds entries,
# re-run is idempotent, no-op outside a git repo.
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

# --- Dry-run path: must announce the entry, must NOT modify .gitignore.
tmp_repo=$(mktemp -d)
trap 'rm -rf "$tmp_repo"' EXIT
git -C "$tmp_repo" init -q
echo "# existing" > "$tmp_repo/.gitignore"
git -C "$tmp_repo" add .gitignore
git -C "$tmp_repo" -c user.email=t@t -c user.name=t commit -q -m init

output=$(cd "$tmp_repo" && bash "$SCRIPT_DIR/install.sh" --dry-run --skill autospec --harness claude 2>&1 || true)
case "$output" in
    *"offer_gitignore"*".autospec/*"*"!.autospec/autospec.yml"*) ;;
    *) echo "FAIL: dry-run did not announce autospec runtime gitignore offer"; echo "$output"; exit 1 ;;
esac
if grep -qxF ".autospec/*" "$tmp_repo/.gitignore"; then
    echo "FAIL: dry-run modified .gitignore"
    exit 1
fi

# --- AUTOSPEC_AUTO_YES path: actually adds the entry on first run, idempotent on re-run.
# Use a fresh HOME so we don't touch the real ~/.claude or ~/.turbo, and use a pre-faked
# turbo repo so bootstrap_turbo does not try to network-clone.
fake_home=$(mktemp -d)
trap 'rm -rf "$tmp_repo" "$fake_home"' EXIT
mkdir -p "$fake_home/bin"
# This test owns only the early gitignore offer. Stop the installer at the
# later runtime-build boundary so a clean Cargo cache cannot turn this focused
# assertion into an eight-minute release build.
printf '%s\n' '#!/bin/sh' 'exit 1' > "$fake_home/bin/cargo"
chmod +x "$fake_home/bin/cargo"
mkdir -p "$fake_home/.turbo"
git -C "$fake_home/.turbo" init -q --bare repo 2>/dev/null || true
mkdir -p "$fake_home/.turbo/repo"
git -C "$fake_home/.turbo/repo" init -q
mkdir -p "$fake_home/.turbo/repo/claude/skills"
git -C "$fake_home/.turbo/repo" -c user.email=t@t -c user.name=t commit -q --allow-empty -m init

HOME="$fake_home" PATH="$fake_home/bin:$PATH" AUTOSPEC_AUTO_YES=1 AUTOSPEC_NO_STAR_PROMPT=1 \
    bash -c "cd '$tmp_repo' && bash '$SCRIPT_DIR/install.sh' --skill autospec --harness claude" >/dev/null 2>&1 || true

if ! grep -qxF ".autospec/*" "$tmp_repo/.gitignore"; then
    echo "FAIL: AUTOSPEC_AUTO_YES run did not add .autospec/*"
    exit 1
fi
if ! grep -qxF "!.autospec/autospec.yml" "$tmp_repo/.gitignore"; then
    echo "FAIL: AUTOSPEC_AUTO_YES run did not unignore .autospec/autospec.yml"
    exit 1
fi

# Idempotency
HOME="$fake_home" PATH="$fake_home/bin:$PATH" AUTOSPEC_AUTO_YES=1 AUTOSPEC_NO_STAR_PROMPT=1 \
    bash -c "cd '$tmp_repo' && bash '$SCRIPT_DIR/install.sh' --skill autospec --harness claude" >/dev/null 2>&1 || true
count=$(grep -cxF ".autospec/*" "$tmp_repo/.gitignore")
if [ "$count" -ne 1 ]; then
    echo "FAIL: .autospec/* duplicated (count=$count)"
    exit 1
fi
exception_count=$(grep -cxF "!.autospec/autospec.yml" "$tmp_repo/.gitignore")
if [ "$exception_count" -ne 1 ]; then
    echo "FAIL: .autospec/autospec.yml exception duplicated (count=$exception_count)"
    exit 1
fi

# --- Outside a git repo: no-op (no crash, no .gitignore created).
outside=$(mktemp -d)
trap 'rm -rf "$tmp_repo" "$fake_home" "$outside"' EXIT
HOME="$fake_home" PATH="$fake_home/bin:$PATH" AUTOSPEC_AUTO_YES=1 AUTOSPEC_NO_STAR_PROMPT=1 \
    bash -c "cd '$outside' && bash '$SCRIPT_DIR/install.sh' --dry-run --skill autospec --harness claude" >/dev/null 2>&1 || true
if [ -f "$outside/.gitignore" ]; then
    echo "FAIL: created .gitignore outside a git repo"
    exit 1
fi

echo "PASS"
