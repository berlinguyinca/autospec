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
import shutil
import subprocess
import sys
import tempfile
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
RUNTIME_SIGNAL_FFI_PATH = "crates/autospec-cli/src/commands/runtime/env.rs"
RUNTIME_SIGNAL_DECLARATION = '''#[cfg(unix)]
extern "C" {
    fn signal(signal: i32, handler: extern "C" fn(i32)) -> usize;
}'''
RUNTIME_SIGNAL_INSTALL = '''unsafe {
        signal(2, record_signal);
        signal(15, record_signal);
    }'''


def approved_unsafe_boundary(relative_path, source, match):
    """Allow only the independently reviewed runtime-signal FFI boundary."""
    if relative_path != RUNTIME_SIGNAL_FFI_PATH:
        return False
    code = rust_code_only(source)
    declaration = rust_code_only(RUNTIME_SIGNAL_DECLARATION)
    installation = rust_code_only(RUNTIME_SIGNAL_INSTALL)
    if code.count(declaration) != 1 or code.count(installation) != 1:
        return False
    declaration_offset = code.index(declaration)
    previous_code = code[:declaration_offset].rstrip()
    if previous_code and previous_code.rsplit("\n", 1)[-1].strip().startswith("#["):
        return False
    candidate = code[match.start():]
    return candidate.startswith(declaration) or candidate.startswith(installation)


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
        code = rust_code_only(source)
        for match in RUST_UNSAFE_SYNTAX.finditer(code):
            line = code.count("\n", 0, match.start()) + 1
            rel = str(path.relative_to(root_path)) if path.is_relative_to(root_path) else str(path)
            if approved_unsafe_boundary(rel, source, match):
                continue
            findings.append({
                "gap_id": f"U{len(findings)+1}",
                "dimension": "unsafe",
                "severity": "must-fix",
                "file": rel,
                "line": line,
                "title": "Unsafe code requires security review",
                "body": "Unsafe code was detected. Replace it with a safe primitive or document an independent security review and invariant proof.",
                "dedupe_key": title_hash(f"unsafe:{rel}:{line}"),
            })
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


def cmd_rank(args):
    rows = read_jsonl(args.findings)
    rows.extend(unsafe_findings(args.root))
    ranked = [normalize(row, i + 1) for i, row in enumerate(rows)]
    ranked.sort(key=lambda r: (-int(r.get("severity_rank", 0)), r.get("file", ""), int(r.get("line") or 0)))
    write_jsonl(args.out, ranked)
    p0 = sum(1 for r in ranked if r.get("priority") == "P0")
    p1 = sum(1 for r in ranked if r.get("priority") == "P1")
    print(f"security rank complete findings={len(ranked)} p0={p0} p1={p1}")
    return 0


def changed_file_root(root, base):
    tmpdir = Path(tempfile.mkdtemp(prefix="security-workstream-diff-"))
    try:
        merge_base = subprocess.run(["git", "-C", root, "merge-base", base, "HEAD"], text=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL).stdout.strip() or base
        names = subprocess.run(["git", "-C", root, "diff", "--name-only", merge_base, "HEAD"], text=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL).stdout.splitlines()
        names += subprocess.run(["git", "-C", root, "ls-files", "--others", "--exclude-standard"], text=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL).stdout.splitlines()
    except Exception:
        return tmpdir
    for rel in sorted(set(n for n in names if n)):
        src = Path(root) / rel
        if src.is_file():
            dst = tmpdir / rel
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(src, dst)
    return tmpdir


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
    unsafe_root = str(changed_file_root(root, args.base)) if args.mode == "pr" else root
    return cmd_rank(argparse.Namespace(findings=str(tmp), root=unsafe_root, out=args.out))


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
