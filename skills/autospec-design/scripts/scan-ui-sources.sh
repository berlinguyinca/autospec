#!/usr/bin/env bash
set -u

repo_root="${1:-}"
if [ -z "$repo_root" ]; then
    printf 'usage: scan-ui-sources.sh <repo-root>\n' >&2
    exit 1
fi
if [ ! -d "$repo_root" ]; then
    printf 'scan-ui-sources: repo root not found: %s\n' "$repo_root" >&2
    exit 1
fi

detect_framework() {
    root="$1"
    if ls "$root"/next.config.* >/dev/null 2>&1; then printf 'next'; return; fi
    if ls "$root"/vite.config.* >/dev/null 2>&1; then printf 'vite'; return; fi
    if [ -f "$root/angular.json" ]; then printf 'angular'; return; fi
    if ls "$root"/svelte.config.* >/dev/null 2>&1; then printf 'svelte'; return; fi
    if [ -f "$root/package.json" ]; then
        if grep -q '"next"' "$root/package.json" 2>/dev/null; then printf 'next'; return; fi
        if grep -q '"vite"' "$root/package.json" 2>/dev/null; then printf 'vite'; return; fi
        if grep -q '"@angular/core"' "$root/package.json" 2>/dev/null; then printf 'angular'; return; fi
        if grep -q '"svelte"' "$root/package.json" 2>/dev/null; then printf 'svelte'; return; fi
    fi
    if find "$root" -type f -name '*.html' -print -quit 2>/dev/null | grep -q .; then
        printf 'vanilla-html'
        return
    fi
    printf 'unknown'
}

json_escape() {
    sed 's/\\/\\\\/g; s/"/\\"/g'
}

framework="$(detect_framework "$repo_root")"

files="$(
    cd "$repo_root" || exit 1
    find . \
        \( -path './node_modules' -o -path './node_modules/*' \
           -o -path './dist' -o -path './dist/*' \
           -o -path './.next' -o -path './.next/*' \
           -o -path './vendor' -o -path './vendor/*' \) -prune \
        -o -type f \( \
            -name '*.tsx' -o -name '*.jsx' -o -name '*.ts' -o -name '*.js' \
            -o -name '*.vue' -o -name '*.svelte' -o -name '*.html' \
            -o -name '*.css' -o -name '*.scss' \
        \) -print \
        | sed 's#^\./##' \
        | sort \
        | head -50
)"

printf '{"framework":"%s","files":[' "$framework"
first=1
while IFS= read -r file; do
    [ -n "$file" ] || continue
    if [ "$first" -eq 0 ]; then printf ','; fi
    first=0
    escaped="$(printf '%s' "$file" | json_escape)"
    printf '"%s"' "$escaped"
done <<EOF
$files
EOF
printf ']}\n'
