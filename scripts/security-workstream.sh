#!/usr/bin/env bash
# scripts/security-workstream.sh — proactive security/compliance workstream.
# Runs/ranks SAST, secret, unsafe, CVE, and dashboard-header findings; emits
# remediation issues for high-severity survivors; and enforces independent
# verifier approval for security fixes.

set -eu

usage() {
    cat <<'USAGE'
Usage:
  security-workstream.sh scan --mode tree|pr [--root DIR] [--base REF] --out FILE
  security-workstream.sh rank --findings FILE [--root DIR] --out FILE
  security-workstream.sh propose-issue --findings FILE --out DIR
  security-workstream.sh check-headers --headers-file FILE
  security-workstream.sh verifier-gate --plan FILE
USAGE
}

if [ $# -eq 0 ] || [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
    usage
    exit 0
fi

cmd="$1"; shift

python3 - "$cmd" "$@" <<'PY'
import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

SENSITIVE_DOMAINS = {"auth", "secrets", "secret", "money", "payments", "payment", "migration", "migrations"}
REQUIRED_HEADERS = {
    "content-security-policy": "Content-Security-Policy",
    "strict-transport-security": "Strict-Transport-Security",
    "x-content-type-options": "X-Content-Type-Options",
    "referrer-policy": "Referrer-Policy",
}


def eprint(*args):
    print(*args, file=sys.stderr)


def read_jsonl(path):
    rows = []
    if not path or not os.path.exists(path):
        return rows
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows


def write_jsonl(path, rows):
    parent = Path(path).parent
    parent.mkdir(parents=True, exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        for row in rows:
            fh.write(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n")


def slug(value):
    return re.sub(r"[^A-Za-z0-9]+", "-", str(value)).strip("-").lower() or "security-finding"


def title_hash(value):
    # Stable enough for dedupe without importing hashlib into shell callers.
    import hashlib
    return "sec-" + hashlib.sha256(str(value).encode("utf-8")).hexdigest()[:16]


RUST_UNSAFE_SYNTAX = re.compile(
    r"\bunsafe\s*(?:\{|fn\b|trait\b|impl\b|extern\b)|#\s*\[\s*unsafe\s*\("
)
# An unsafe boundary carries its own invariant proof: `// SAFETY:` beside the block, or a
# `/// # Safety` section on an `unsafe fn`. Recognised on the block's own line or the comment
# run immediately above it, so the proof and the code it justifies move together in one diff.
# `SECURITY:` is accepted because the runtime signal handlers already use it that way.
UNSAFE_RATIONALE = re.compile(r"SAFETY:|SECURITY:|#\s*Safety")


def approved_unsafe_boundary(relative_path, source, match):
    """Allow only unsafe boundaries whose invariant is recorded beside them."""
    lines = source.split("\n")
    index = source[:match.start()].count("\n")
    run, cursor = [lines[index]], index - 1
    while cursor >= 0 and index - cursor <= 10:
        stripped = lines[cursor].strip()
        if not (stripped.startswith("//") or stripped.startswith("#[") or stripped == ""):
            break
        run.append(stripped)
        cursor -= 1
    if UNSAFE_RATIONALE.search("\n".join(run)):
        return True
    code = rust_code_only(source)
    function_matches = list(re.finditer(r"\bfn\s+([A-Za-z0-9_]+)\s*\(", source[:match.start()]))
    function_match = function_matches[-1] if function_matches else None
    function_name = function_match.group(1) if function_match else ""
    function_end = None
    if function_match:
        function_open = code.find("{", function_match.end())
        function_depth = 0
        for index in range(function_open, len(code)):
            if code[index] == "{":
                function_depth += 1
            elif code[index] == "}":
                function_depth -= 1
                if function_depth == 0:
                    function_end = index + 1
                    break
    open_brace = code.find("{", match.start())
    depth = 0
    end = None
    for index in range(open_brace, len(code)):
        if code[index] == "{":
            depth += 1
        elif code[index] == "}":
            depth -= 1
            if depth == 0:
                end = index + 1
                break
    body = " ".join(code[match.start():end].split()) if end else ""
    reviewed = {
        ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests.rs", "autonomous_executor_bridge_capture_and_reap_failure_retains_exact_quarantine"): "unsafe { nix::libc::prctl( nix::libc::PR_GET_CHILD_SUBREAPER, std::ptr::addr_of_mut!(subreaper), 0, 0, 0, ) }",
        ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests.rs", "autonomous_executor_bridge_clean_supervision_restores_prior_subreaper_state"): "unsafe { nix::libc::prctl( nix::libc::PR_GET_CHILD_SUBREAPER, std::ptr::addr_of_mut!(observed), 0, 0, 0, ) }",
        ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests.rs", "autonomous_executor_bridge_clean_supervision_preserves_enabled_subreaper_state"): "unsafe { nix::libc::prctl( nix::libc::PR_GET_CHILD_SUBREAPER, std::ptr::addr_of_mut!(observed), 0, 0, 0, ) }",
        ("crates/autospec-cli/src/commands/runtime/env/session.rs", "verify_active"): "unsafe { nix::libc::geteuid() }",
        ("crates/autospec-cli/src/commands/runtime/env/worker.rs", "capture"): "unsafe { nix::libc::tcgetpgrp(nix::libc::STDIN_FILENO) }",
        ("crates/autospec-cli/src/commands/runtime/env/worker.rs", "configure_process_group"): "unsafe { command.pre_exec(setup_foreground_child); }",
        ("crates/autospec-cli/src/commands/runtime/env/worker.rs", "set_terminal_foreground"): "unsafe { nix::libc::tcsetpgrp(nix::libc::STDIN_FILENO, process_group) }",
    }
    expected_body = reviewed.get((re.sub(r"(/executor_bridge/tests)/.+", r"\1.rs", relative_path), function_name))
    normalized_function = (
        " ".join(code[function_match.start():function_end].split())
        if function_match and function_end and match.start() < function_end
        else ""
    )
    # The pinned allowlist stays: it approves a boundary by exact body, so editing reviewed
    # code revokes its approval. The bespoke runtime-signal case that used to sit here is
    # gone — those two sites carry `// SAFETY:` and `// SECURITY:` comments, so the rule above
    # covers them without a path-specific branch.
    return body == expected_body and normalized_function.count(expected_body) == 1


def blank_non_code(source, output, start, end):
    for index in range(start, end):
        output[index] = "\n" if source[index] == "\n" else " "


def raw_string_end(source, start):
    if start and (source[start - 1].isalnum() or source[start - 1] == "_"):
        return None
    prefix = 2 if source.startswith("br", start) else 1 if source.startswith("r", start) else 0
    if not prefix:
        return None
    quote = start + prefix
    while quote < len(source) and source[quote] == "#":
        quote += 1
    if quote >= len(source) or source[quote] != '"':
        return None
    terminator = '"' + source[start + prefix:quote]
    end = source.find(terminator, quote + 1)
    return len(source) if end < 0 else end + len(terminator)


def rust_code_only(source):
    output = list(source)
    index = 0
    while index < len(source):
        if source.startswith("//", index):
            end = source.find("\n", index)
            end = len(source) if end < 0 else end
            blank_non_code(source, output, index, end)
            index = end
            continue
        if source.startswith("/*", index):
            end = index + 2
            depth = 1
            while end < len(source) and depth:
                if source.startswith("/*", end):
                    depth += 1
                    end += 2
                elif source.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
            blank_non_code(source, output, index, end)
            index = end
            continue
        end = raw_string_end(source, index)
        if end is not None:
            blank_non_code(source, output, index, end)
            index = end
            continue
        if source[index] == '"':
            end = index + 1
            while end < len(source):
                if source[end] == "\\":
                    end += 2
                elif source[end] == '"':
                    end += 1
                    break
                else:
                    end += 1
            blank_non_code(source, output, index, min(end, len(source)))
            index = end
            continue
        index += 1
    return "".join(output)


def unsafe_findings_for_source(relative_path, source):
    findings = []
    code = rust_code_only(source)
    for match in RUST_UNSAFE_SYNTAX.finditer(code):
        unsafe_offset = code.find("unsafe", match.start(), match.end())
        line = code.count("\n", 0, unsafe_offset) + 1
        if approved_unsafe_boundary(relative_path, source, match):
            continue
        syntax = (
            "attribute"
            if match.group(0).lstrip().startswith("#")
            else re.search(
                r"\bunsafe\s*(\{|fn\b|trait\b|impl\b|extern\b)",
                match.group(0),
            ).group(1)
        )
        findings.append({
            "gap_id": f"U{len(findings)+1}",
            "dimension": "unsafe",
            "severity": "must-fix",
            "file": relative_path,
            "line": line,
            "title": "Unsafe code requires security review",
            "body": "Unsafe code was detected. Replace it with a safe primitive or document an independent security review and invariant proof.",
            "dedupe_key": title_hash(f"unsafe:{relative_path}:{line}"),
            "_unsafe_syntax": syntax,
        })
    return findings


def unsafe_findings(root):
    root_path = Path(root or ".")
    findings = []
    for path in root_path.rglob("*"):
        if not path.is_file() or ".git" in path.parts:
            continue
        if path.suffix != ".rs":
            continue
        try:
            lines = path.read_text(errors="ignore").splitlines()
        except OSError:
            continue
        source = "\n".join(lines)
        rel = str(path.relative_to(root_path)) if path.is_relative_to(root_path) else str(path)
        findings.extend(unsafe_findings_for_source(rel, source))
    for index, finding in enumerate(findings, start=1):
        finding["gap_id"] = f"U{index}"
    return findings


def score(row):
    dim = str(row.get("dimension", "")).lower()
    severity = str(row.get("severity", "")).lower()
    title = str(row.get("title", ""))
    body = str(row.get("body", ""))
    text = f"{title} {body}".lower()

    exploitability = 2
    exposure = 2
    if severity == "must-fix":
        exploitability += 2
        exposure += 1
    if dim in {"secrets", "secret"}:
        exploitability = 5
        exposure = 5
    elif dim in {"injection", "vuln"}:
        exploitability = max(exploitability, 4)
        exposure = max(exposure, 3)
    elif dim == "unsafe":
        exploitability = max(exploitability, 4)
        exposure = max(exposure, 2)
    elif dim == "cve":
        exploitability = max(exploitability, 3)
        exposure = max(exposure, 3)
    elif dim == "headers":
        exploitability = max(exploitability, 3)
        exposure = max(exposure, 4)
    if any(word in text for word in ["critical", "rce", "remote code", "credential", "token", "password"]):
        exploitability = max(exploitability, 5)
    if any(word in text for word in ["dashboard", "public", "internet", "api", "web"]):
        exposure = max(exposure, 4)

    severity_rank = exploitability * exposure
    if dim in {"secrets", "secret"} or severity_rank >= 20:
        priority = "P0"
    elif severity == "must-fix" or severity_rank >= 16:
        priority = "P1"
    else:
        priority = "P2"
    return exploitability, exposure, severity_rank, priority


def normalize(row, idx):
    out = dict(row)
    out.pop("_unsafe_syntax", None)
    out.setdefault("gap_id", f"G{idx}")
    out.setdefault("dimension", "vuln")
    out.setdefault("severity", "nice-to-have")
    out.setdefault("file", "")
    out.setdefault("line", 0)
    out.setdefault("title", "Security finding")
    out.setdefault("body", "Review and remediate the security finding.")
    out.setdefault("dedupe_key", title_hash(f"{out['dimension']}:{out['file']}:{out['title']}"))
    exploitability, exposure, rank, priority = score(out)
    out["exploitability"] = exploitability
    out["exposure"] = exposure
    out["severity_rank"] = rank
    out["priority"] = priority
    out["remediation"] = remediation(out)
    return out


def remediation(row):
    dim = str(row.get("dimension", ""))
    if dim in {"secrets", "secret"}:
        return "Remove the secret from code, rotate the credential, and add a regression secret scan."
    if dim == "unsafe":
        return "Replace unsafe code with a safe primitive or require independent verifier approval with an invariant proof."
    if dim == "headers":
        return "Restore the dashboard security header baseline: CSP with frame-ancestors, HSTS, nosniff, and Referrer-Policy."
    if dim == "cve":
        return "Dedupe against the debt/CVE workstream, then upgrade or pin the vulnerable dependency once."
    if dim == "injection":
        return "Validate input at the boundary and use structured APIs instead of string concatenation/eval."
    return "Triage exploitability and exposure, apply the smallest remediation, then re-run the security workstream."


def write_ranked(rows, out):
    ranked = [normalize(row, i + 1) for i, row in enumerate(rows)]
    ranked.sort(key=lambda r: (-int(r.get("severity_rank", 0)), r.get("file", ""), int(r.get("line") or 0)))
    write_jsonl(out, ranked)
    p0 = sum(1 for r in ranked if r.get("priority") == "P0")
    p1 = sum(1 for r in ranked if r.get("priority") == "P1")
    print(f"security rank complete findings={len(ranked)} p0={p0} p1={p1}")
    return 0


def cmd_rank(args):
    rows = read_jsonl(args.findings)
    if args.base:
        unsafe_rows, error = unsafe_findings_pr(args.root, args.base)
        if error:
            eprint(error)
            return 2
        rows.extend(unsafe_rows)
    else:
        rows.extend(unsafe_findings(args.root))
    return write_ranked(rows, args.out)


def run_git(root, args):
    result = subprocess.run(
        ["git", "-C", root, *args],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or "git command failed"
        raise RuntimeError(f"git {' '.join(args)}: {detail}")
    return result.stdout


def changed_rust_paths(root, merge_base):
    fields = run_git(
        root,
        ["diff", "--name-status", "-z", "--find-renames", merge_base, "HEAD", "--", "*.rs"],
    ).split("\0")
    paths = []
    index = 0
    while index < len(fields) and fields[index]:
        status = fields[index]
        index += 1
        if status.startswith(("R", "C")):
            old_path, current_path = fields[index:index + 2]
            index += 2
        else:
            old_path = current_path = fields[index]
            index += 1
        paths.append((status[0], old_path, current_path))
    untracked = run_git(
        root,
        ["ls-files", "-z", "--others", "--exclude-standard", "--", "*.rs"],
    ).split("\0")
    paths.extend(("?", "", path) for path in untracked if path)
    return paths


def added_unsafe_token_lines(root, merge_base, old_path, current_path):
    pathspec = list(dict.fromkeys(path for path in (old_path, current_path) if path))
    diff = run_git(
        root,
        [
            "diff",
            "--word-diff=porcelain",
            "--word-diff-regex=[A-Za-z_][A-Za-z0-9_]*|[^[:space:]]",
            "--find-renames",
            "--unified=0",
            merge_base,
            "HEAD",
            "--",
            *pathspec,
        ],
    )
    counts = {}
    current_line = None
    has_current_content = False
    for line in diff.splitlines():
        hunk = re.match(r"^@@ -\d+(?:,\d+)? \+(\d+)", line)
        if hunk:
            current_line = int(hunk.group(1))
            has_current_content = False
        elif current_line is not None and line == "~":
            if has_current_content:
                current_line += 1
            has_current_content = False
        elif current_line is not None and line.startswith("+") and not line.startswith("+++"):
            has_current_content = True
            added = len(re.findall(r"\bunsafe\b", line[1:]))
            if added:
                counts[current_line] = counts.get(current_line, 0) + added
        elif current_line is not None and not line.startswith("-"):
            has_current_content = True
    raw_diff = run_git(
        root,
        [
            "diff",
            "--unified=0",
            "--find-renames",
            merge_base,
            "HEAD",
            "--",
            *pathspec,
        ],
    )
    deleted = {}
    for line in raw_diff.splitlines():
        if line.startswith("-") and not line.startswith("---") and re.search(r"\bunsafe\b", line):
            fingerprint = " ".join(line[1:].split())
            deleted[fingerprint] = deleted.get(fingerprint, 0) + 1
    return counts, deleted


def unsafe_findings_pr(root, base):
    try:
        merge_base = run_git(root, ["merge-base", base, "HEAD"]).strip()
        if not merge_base:
            raise RuntimeError("git merge-base returned no commit")
        findings = []
        for status, old_path, current_path in changed_rust_paths(root, merge_base):
            current_file = Path(root) / current_path
            if status == "D" or not current_file.is_file():
                continue
            current_source = current_file.read_text(errors="ignore")
            current_lines = current_source.splitlines()
            current = unsafe_findings_for_source(
                current_path,
                current_source,
            )
            if status == "?":
                findings.extend(current)
                continue
            added_lines, deleted = added_unsafe_token_lines(
                root,
                merge_base,
                old_path,
                current_path,
            )
            for finding in current:
                line = finding["line"]
                if added_lines.get(line, 0):
                    added_lines[line] -= 1
                    fingerprint = " ".join(current_lines[line - 1].split())
                    if deleted.get(fingerprint, 0):
                        deleted[fingerprint] -= 1
                    else:
                        findings.append(finding)
        for index, finding in enumerate(findings, start=1):
            finding["gap_id"] = f"U{index}"
        return findings, None
    except (OSError, RuntimeError) as error:
        return [], f"unsafe PR scan failed closed: {error}"


def cmd_scan(args):
    root = args.root or "."
    tmp = Path(args.out).with_suffix(".raw.jsonl")
    script = Path(root) / "skills" / "autospec-shared" / "scripts" / "security-scan.sh"
    if not script.exists():
        script = Path(__file__).resolve().parent.parent / "skills" / "autospec-shared" / "scripts" / "security-scan.sh"
    scan_args = ["bash", str(script)]
    if args.mode == "pr":
        scan_args += ["--diff", args.base or "origin/main", "--root", root]
    else:
        scan_args += ["--tree", "--root", root]
    result = subprocess.run(scan_args, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.stderr:
        eprint(result.stderr.rstrip())
    if result.returncode == 2:
        eprint("security scan failed closed")
        return 2
    tmp.parent.mkdir(parents=True, exist_ok=True)
    tmp.write_text(result.stdout, encoding="utf-8")
    rows = read_jsonl(tmp)
    if args.mode == "pr":
        unsafe_rows, error = unsafe_findings_pr(root, args.base)
        if error:
            eprint(error)
            return 2
        rows.extend(unsafe_rows)
    else:
        rows.extend(unsafe_findings(root))
    return write_ranked(rows, args.out)


def issue_body(row):
    title = row["title"].rstrip(".")
    file_line = f"{row.get('file','')}:{row.get('line',0)}"
    cmd = "bash scripts/security-workstream.sh rank --findings .autospec/security/security-findings.jsonl --out .autospec/security/security-ranked.jsonl"
    return f"""## Goal
Remediate `{title}` at `{file_line}` and prove the security workstream no longer reports dedupe key `{row['dedupe_key']}`.

## Acceptance criteria
- [ ] `{file_line}` no longer triggers `{row['dedupe_key']}` in `security-workstream.sh` output.
- [ ] `.autospec/security/security-ranked.jsonl` has no `P0`/`P1` row for `{row['dedupe_key']}`.
- [ ] `security verifier gate` records an independent verifier different from the fix author.

## Files to read first
- `{row.get('file','')}`
- `scripts/security-workstream.sh`

## Dependencies
none

## Implementation outline
- Inspect `{file_line}` and apply the smallest remediation for `{row.get('dimension','vuln')}`.
- Remediation guidance: {row.get('remediation','Re-run the scan and remediate the finding.')}
- Dedupe with existing debt/CVE workstream using dedupe_key `{row['dedupe_key']}` before filing another dependency issue.

## Tests required
- smoke

### Primary smoke test (inner loop)
```bash
{cmd}
```

---
- labels: auto-implement,security,priority:high
- priority: {row.get('priority','P1')}
- severity_rank: {row.get('severity_rank',0)}
- exploitability: {row.get('exploitability',0)}
- exposure: {row.get('exposure',0)}
- dedupe_key: {row['dedupe_key']}
"""


def cmd_propose(args):
    outdir = Path(args.out)
    outdir.mkdir(parents=True, exist_ok=True)
    rows = read_jsonl(args.findings)
    selected = [r for r in rows if r.get("priority") in {"P0", "P1"} or r.get("severity") == "must-fix"]
    for row in selected:
        row = normalize(row, 1)
        path = outdir / f"{str(row.get('priority','p1')).lower()}-{slug(row.get('title','security-finding'))}.md"
        path.write_text(issue_body(row), encoding="utf-8")
    print(f"security issue proposals written={len(selected)} dir={outdir}")
    return 0


def parse_headers(path):
    headers = {}
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or line.startswith("/") or ":" not in line:
            continue
        key, value = line.split(":", 1)
        headers[key.strip().lower()] = value.strip()
    return headers


def cmd_headers(args):
    headers = parse_headers(args.headers_file)
    failures = []
    for key, label in REQUIRED_HEADERS.items():
        if key not in headers:
            failures.append(f"missing {label}")
    csp = headers.get("content-security-policy", "")
    if "frame-ancestors" not in csp.lower():
        failures.append("missing frame-ancestors in Content-Security-Policy")
    if headers.get("x-content-type-options", "").lower() != "nosniff":
        failures.append("X-Content-Type-Options must be nosniff")
    if "max-age=" not in headers.get("strict-transport-security", "").lower():
        failures.append("Strict-Transport-Security must include max-age")
    if not headers.get("referrer-policy", ""):
        failures.append("Referrer-Policy must be non-empty")
    if failures:
        for failure in failures:
            print(f"security header baseline failed: {failure}")
        return 1
    print("security header baseline passed")
    return 0


def cmd_verifier(args):
    plan = json.loads(Path(args.plan).read_text(encoding="utf-8"))
    author = str(plan.get("author", "")).strip()
    verifier = str(plan.get("verifier", "")).strip()
    human = str(plan.get("human_approved_by", "")).strip()
    domains = {str(d).lower() for d in plan.get("touched_domains", [])}
    failures = []
    if not verifier or verifier == author:
        failures.append("independent verifier required")
    sensitive = sorted(d for d in domains if d in SENSITIVE_DOMAINS)
    if sensitive and not human:
        failures.append("human gate required: " + ",".join(sensitive))
    if not str(plan.get("evidence", "")).strip():
        failures.append("verification evidence required")
    if failures:
        for failure in failures:
            print(f"security verifier gate failed: {failure}")
        return 1
    print(f"security verifier gate passed verifier={verifier} human={human or 'n/a'}")
    return 0


def main():
    cmd = sys.argv[1]
    parser = argparse.ArgumentParser(prog=f"security-workstream.sh {cmd}")
    if cmd == "rank":
        parser.add_argument("--findings", required=True)
        parser.add_argument("--root", default=".")
        parser.add_argument("--base")
        parser.add_argument("--out", required=True)
        return cmd_rank(parser.parse_args(sys.argv[2:]))
    if cmd == "scan":
        parser.add_argument("--mode", choices=["tree", "pr"], required=True)
        parser.add_argument("--root", default=".")
        parser.add_argument("--base", default="origin/main")
        parser.add_argument("--out", required=True)
        return cmd_scan(parser.parse_args(sys.argv[2:]))
    if cmd == "propose-issue":
        parser.add_argument("--findings", required=True)
        parser.add_argument("--out", required=True)
        return cmd_propose(parser.parse_args(sys.argv[2:]))
    if cmd == "check-headers":
        parser.add_argument("--headers-file", required=True)
        return cmd_headers(parser.parse_args(sys.argv[2:]))
    if cmd == "verifier-gate":
        parser.add_argument("--plan", required=True)
        return cmd_verifier(parser.parse_args(sys.argv[2:]))
    eprint(f"security-workstream.sh: unknown command: {cmd}")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
PY
