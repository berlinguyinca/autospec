#!/usr/bin/env bash
# scripts/lint-cfg-test-branches.sh — reject `#[cfg(test)]` inside function
# bodies, function parameters, and struct/enum/union definitions in
# crates/*/src/** (issue #4066).
#
# A `#[cfg(test)]` branch inside an always-compiled function is a test seam
# welded into production code: the branch exists in one build and not the
# other, so the function under test is not the function that ships. The
# boundary-seam replacement is a trait, an injected collaborator, or a
# parameter that exists in both builds (module-level `#[cfg(test)]` items —
# the `tests` module, test-only statics — are the normal shape and are
# exempt).
#
# The detector is a character-level scanner (not a text grep): string and
# char literal contents, raw strings, and comments are stripped before any
# brace/paren counting, and a context stack tracks the innermost `fn` /
# `struct` / `enum` / `union` / `impl` so that a `#[cfg(test)]` attribute is
# flagged exactly when its enclosing context is a function or a type.
#
# Baseline ratchet (shrink-only): existing sites are listed in
# config/cfg-test-branches-baseline.tsv (override with
# $AUTOSPEC_CFG_TEST_BRANCH_BASELINE), one line per site:
#   <path>\t<enclosing fn/type>\t<hash12 of the next non-attribute line>
# A site present in the current scan AND the baseline is an advisory
# INFO:CFG_TEST_BRANCH:... line (audit trail, does not block). A site NOT in
# the baseline is a finding. Baseline entries whose site no longer exists are
# removed from the baseline by this script (the file is rewritten in place;
# the ratchet only ever shrinks). New sites are never added automatically —
# replacing the site with a boundary seam is the fix.
#
# Waiver: the `#[cfg(test)]` attribute line or the line immediately above it
# may carry `// linter:allow-CFG_TEST_BRANCH <reason>`. The reason is
# mandatory; a bare marker is rejected and the site stays flagged. Waived
# sites emit an advisory INFO:CFG_TEST_BRANCH:... line for audit.
#
# Usage:
#   scripts/lint-cfg-test-branches.sh [PATH...]
#   scripts/lint-cfg-test-branches.sh --list [PATH...]
#   scripts/lint-cfg-test-branches.sh --help
#
# With no arguments the scan covers every *.rs under crates/*/src/.
# PATH may be a .rs file or a directory (scanned recursively for *.rs).
#
# --list: emit one INFO:CFG_TEST_BRANCH:<path>:<line>: line per current site
# (baseline and waivers are not consulted) and always exit 0. Audit mode.
#
# Output: one finding per line on stdout:
#   CFG_TEST_BRANCH:<path>:<line>: in <FN|TYPE> '<name>': replace the
#   test-only branch with a boundary seam (trait, injected collaborator, or
#   a parameter that exists in both builds)
#
# Exit code = number of findings (0 = pass), capped at 64.

set -eu

SCRIPT_PATH="$0"
SCRIPT_DIR=$(cd "$(dirname "$SCRIPT_PATH")" && pwd)
ROOT_DIR=$(cd "$SCRIPT_DIR/.." && pwd)
BASELINE="${AUTOSPEC_CFG_TEST_BRANCH_BASELINE:-$ROOT_DIR/config/cfg-test-branches-baseline.tsv}"

MODE="check"
if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
    sed -n '2,/^set -eu/p' "$SCRIPT_PATH" | sed '$d' | sed 's/^# \{0,1\}//'
    exit 0
fi
if [[ "${1:-}" == "--list" ]]; then
    MODE="list"
    shift
fi

FILES=()
if [[ $# -gt 0 ]]; then
    for p in "$@"; do
        if [[ -f "$p" && "$p" == *.rs ]]; then
            FILES+=("$p")
        elif [[ -d "$p" ]]; then
            while IFS= read -r f; do FILES+=("$f"); done < <(find "$p" -name '*.rs' -type f | sort)
        else
            echo "WARN: skipping non-source path: $p" >&2
        fi
    done
else
    while IFS= read -r f; do FILES+=("$f"); done < <(
        find "$ROOT_DIR"/crates -mindepth 3 -path '*/src/*.rs' -type f 2>/dev/null | sort
    )
fi

if [[ ${#FILES[@]} -eq 0 ]]; then
    echo "INFO:CFG_TEST_BRANCH: no .rs files to scan"
    exit 0
fi

python3 - "$MODE" "$ROOT_DIR" "$BASELINE" "${FILES[@]}" <<'PY'
import hashlib
import os
import re
import sys

MODE, ROOT_DIR, BASELINE = sys.argv[1], sys.argv[2], sys.argv[3]
FILES = sys.argv[4:]

Q = chr(39)
FN_RE = re.compile(r'(?<![\w!])(fn)\s+([A-Za-z_][A-Za-z0-9_]*)')
TYPE_RE = re.compile(r'^\s*(pub(\([^)]*\))?\s+)?(struct|enum|union)\s+([A-Za-z_][A-Za-z0-9_]*)')
IMPL_RE = re.compile(r'^\s*(pub(\([^)]*\))?\s+)?impl\b')
CFG_PREFIX_RE = re.compile(r'^#\[\s*cfg\(\s*test\s*\)\s*\]')
ALLOW_MARKER = 'linter:allow-CFG_TEST_BRANCH'

ALNUM = set(
    'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_'
)


class Ctx(object):
    __slots__ = ('kind', 'name', 'dop', 'body', 'popen')

    def __init__(self, kind, name, d, p):
        self.kind = kind
        self.name = name
        self.dop = d
        self.body = False
        self.popen = p


def clean_line(line, state):
    """Strip string/char literal contents and comments; return code text."""
    out = []
    j = 0
    while j < len(line):
        c = line[j]
        if state['in_str']:
            if state['raw'] >= 0:
                if c == '"':
                    k = j + 1
                    h = 0
                    while k < len(line) and line[k] == '#':
                        h += 1
                        k += 1
                    if h >= state['raw']:
                        state['in_str'] = False
                        state['raw'] = -1
                        j = k
                        continue
                j += 1
                continue
            if c == '\\' and j + 1 < len(line):
                j += 2
                continue
            if c == '"':
                state['in_str'] = False
            j += 1
            continue
        if state['in_char']:
            if c == '\\' and j + 1 < len(line):
                j += 2
                continue
            if c == Q:
                state['in_char'] = False
            j += 1
            continue
        if c == '"':
            # Raw string prefix: r or br, optionally followed by #'s.
            h = 0
            k = len(out) - 1
            while k >= 0 and out[k] == '#':
                h += 1
                k -= 1
            if (
                k >= 0
                and out[k] == 'r'
                and (k == 0 or out[k - 1] not in ALNUM or out[k - 1] == 'b')
            ):
                state['in_str'] = True
                state['raw'] = h
                j += 1
                continue
            state['in_str'] = True
            state['raw'] = -1
            j += 1
            continue
        if c == Q:
            if j + 1 < len(line) and line[j + 1] == '\\':
                e = j + 2
                if e < len(line) and line[e] == 'x' and e + 2 < len(line):
                    e += 3
                else:
                    e += 1
                if e < len(line) and line[e] == Q:
                    j = e + 1
                    continue
            elif j + 2 < len(line) and line[j + 1] != Q and line[j + 2] == Q:
                state['in_char'] = True
                j += 1
                continue
        if c == '/' and j + 1 < len(line) and line[j + 1] == '/':
            break
        if c == '/' and j + 1 < len(line) and line[j + 1] == '*':
            state['in_block'] = True
            j += 2
            continue
        out.append(c)
        j += 1
    return ''.join(out)


def waiver_reason(line):
    """Return the waiver reason on `line`, or '' (bare marker -> '')."""
    idx = line.find(ALLOW_MARKER)
    if idx < 0:
        return ''
    rest = line[idx + len(ALLOW_MARKER):].strip()
    rest = re.sub(r'^//[ \t]*', '', rest).strip()
    return rest


def scan(path):
    """Return (sites, waiver_by_line, state, final_depth, leftover_ctx).

    Each site is (lineno, kind, name, hash12, raw_line). hash12 is the
    sha256 prefix of the next non-attribute, non-comment, non-blank line
    after the #[cfg(test)] attribute — stable under line-number drift.
    """
    with open(path) as fh:
        lines = fh.read().split('\n')
    stack = []
    depth = 0
    pdepth = 0
    state = {'in_str': False, 'in_char': False, 'in_block': False, 'raw': -1}
    sites = []
    waivers = {}
    for i, raw in enumerate(lines):
        line = raw
        if state['in_block']:
            e = line.find('*/')
            if e == -1:
                continue
            line = ' ' * (e + 2) + line[e + 2:]
            state['in_block'] = False
        line = clean_line(line, state)
        t = line.strip()
        opens = []
        m = FN_RE.search(line)
        if m:
            pre = line[: m.start()]
            if not pre or pre[-1] in ' \t({=&|!':
                opens.append(('FN', m.group(2)))
        mt = TYPE_RE.match(line)
        if mt and not any(k.kind == 'FN' for k in stack):
            opens.append(('TYPE', mt.group(4)))
        if IMPL_RE.match(line):
            opens.append(('IMPL', '<impl>'))
        for k, nm in opens:
            stack.append(Ctx(k, nm, depth, pdepth))
        if CFG_PREFIX_RE.match(t):
            waivers[i] = waiver_reason(raw) or waiver_reason(lines[i - 1]) if i > 0 else waiver_reason(raw)
            if stack and stack[-1].kind in ('FN', 'TYPE'):
                # Hash the next line of real code after attribute/comment run.
                j = i + 1
                while j < len(lines):
                    nx = lines[j].strip()
                    if (
                        nx
                        and not nx.startswith('#[')
                        and not nx.startswith('//')
                    ):
                        break
                    j += 1
                h = hashlib.sha256(
                    lines[j].strip().encode('utf-8')
                ).hexdigest()[:12] if j < len(lines) else 'eof'
                sites.append((i + 1, stack[-1].kind, stack[-1].name, h, t))
        depth_n = depth + line.count('{') - line.count('}')
        pdepth_n = pdepth + line.count('(') - line.count(')')
        if stack and not stack[-1].body and stack[-1].kind in ('FN', 'TYPE'):
            c = stack[-1]
            if ';' in line and depth_n == c.dop and pdepth_n <= c.popen:
                stack.pop()
        if stack:
            c = stack[-1]
            if not c.body and (
                depth_n > c.dop
                or ('{' in line and depth == c.dop and pdepth <= c.popen)
            ):
                c.body = True
            if c.body and depth_n <= c.dop:
                stack.pop()
        depth, pdepth = depth_n, pdepth_n
    return sites, waivers, state, depth, len(stack)


def rel(path):
    try:
        return os.path.relpath(path, ROOT_DIR)
    except ValueError:
        return path


# ---- scan all files -------------------------------------------------------
all_sites = []  # (relpath, lineno, kind, name, hash, waiver)
for p in FILES:
    sites, waivers, state, depth, nstack = scan(p)
    r = rel(p)
    for ln, kind, name, h, t in sites:
        all_sites.append((r, ln, kind, name, h, waivers.get(ln - 1, '')))
    if depth != 0 or nstack:
        sys.stderr.write(
            '# WARN CFG_TEST_BRANCH: scanner desync in %s '
            '(final_depth=%d leftover_ctx=%d in_str=%s in_block=%s); '
            'findings for this file may be unreliable\n'
            % (r, depth, nstack, state['in_str'], state['in_block'])
        )

# ---- list mode -------------------------------------------------------------
if MODE == 'list':
    for r, ln, kind, name, h, w in all_sites:
        print('INFO:CFG_TEST_BRANCH:%s:%d: in %s %r (hash %s)' % (r, ln, kind, name, h))
    sys.exit(0)

# ---- baseline ratchet (shrink-only) ---------------------------------------
# The baseline is keyed per-file; entries for files not in the current scan
# scope (e.g. an explicit file-argument run) are preserved verbatim, never
# treated as stale. Only entries for scanned files whose site is gone are
# removed.
rel_paths = [rel(p) for p in FILES]
scanned = set(rel_paths)
baseline_entries = []   # list of (key, in_scope) preserving file order
if os.path.exists(BASELINE):
    with open(BASELINE) as fh:
        for ln in fh:
            ln = ln.rstrip('\n')
            if not ln or ln.startswith('#'):
                continue
            parts = ln.split('\t')
            if len(parts) >= 3:
                baseline_entries.append(((parts[0], parts[1], parts[2]), True))
for i, (key, _) in enumerate(baseline_entries):
    if key[0] not in scanned:
        baseline_entries[i] = (key, False)

findings = []
remaining = {}
for key, in_scope in baseline_entries:
    if in_scope:
        remaining[key] = remaining.get(key, 0) + 1
for r, ln, kind, name, h, waiver in all_sites:
    if waiver:
        print(
            'INFO:CFG_TEST_BRANCH:%s:%d: waived (%s): in %s %r: '
            'replace the test-only branch with a boundary seam'
            % (r, ln, waiver, kind, name)
        )
        continue
    key = (r, name, h)
    if remaining.get(key, 0) > 0:
        remaining[key] -= 1
        print(
            'INFO:CFG_TEST_BRANCH:%s:%d: baselined (ratchet): in %s %r: '
            'replace the test-only branch with a boundary seam'
            % (r, ln, kind, name)
        )
    else:
        findings.append((r, ln, kind, name))

for r, ln, kind, name in findings:
    print(
        'CFG_TEST_BRANCH:%s:%d: in %s %r: replace the test-only branch '
        'with a boundary seam (trait, injected collaborator, or a parameter '
        'that exists in both builds)' % (r, ln, kind, name)
    )

# ---- rewrite baseline: keep matched + out-of-scope entries (shrink-only) --
# After the check loop above, remaining[key] = baseline entries for which no
# current site matched. Those are exactly the stale entries to drop.
stale_count = dict(remaining)
stale = []
if os.path.exists(BASELINE):
    kept_lines = []
    for key, in_scope in baseline_entries:
        if not in_scope:
            kept_lines.append('\t'.join(key))          # out of scan scope: preserve
        elif stale_count.get(key, 0) > 0:
            stale_count[key] -= 1
            stale.append(key)                          # scanned file, site gone
        else:
            kept_lines.append('\t'.join(key))          # still present: keep
    if stale:
        tmp = BASELINE + '.tmp'
        with open(tmp, 'w') as fh:
            fh.write('# cfg-test-branches baseline (shrink-only ratchet, issue #4066)\n')
            fh.write('# format: <path>\\t<enclosing fn/type>\\t<hash12 of next code line>\n')
            for line in kept_lines:
                fh.write(line + '\n')
        os.replace(tmp, BASELINE)
    for key in stale:
        print('INFO:CFG_TEST_BRANCH:baseline: removed stale entry: %s' % '\t'.join(key))

sys.exit(min(len(findings), 64))
PY
