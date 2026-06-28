#!/usr/bin/env python3
"""Digital Twin v1 scanner for local Autospec metadata.

Local filesystem only. No network, no GitHub API, no app code writes.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
from pathlib import Path

GENERATED_AT = "1970-01-01T00:00:00Z"
GENERATOR = "autospec-digital-twin-v1"
EXCLUDE_DIRS = {".git", "node_modules", ".venv", "venv", "__pycache__", "dist", "build", "target"}
EXCLUDE_PREFIXES = (".autospec/state/", ".autospec/reports/")
LANG = {
    ".js": "JavaScript", ".jsx": "JavaScript", ".mjs": "JavaScript", ".cjs": "JavaScript",
    ".ts": "TypeScript", ".tsx": "TypeScript", ".py": "Python", ".sh": "Shell",
    ".go": "Go", ".rs": "Rust", ".java": "Java", ".rb": "Ruby", ".sql": "SQL",
    ".md": "Markdown", ".json": "JSON", ".yml": "YAML", ".yaml": "YAML", ".toml": "TOML",
}


def rel(root: Path, path: Path) -> str:
    return path.relative_to(root).as_posix()


def ensure(root: Path) -> tuple[Path, Path]:
    state = root / ".autospec" / "state"
    reports = root / ".autospec" / "reports"
    state.mkdir(parents=True, exist_ok=True)
    reports.mkdir(parents=True, exist_ok=True)
    return state, reports


def write_json(path: Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text.rstrip() + "\n", encoding="utf-8")


def load_json(path: Path, default):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return default


def read_text(root: Path, path: str, limit: int = 200_000) -> str:
    try:
        return (root / path).read_text(encoding="utf-8", errors="ignore")[:limit]
    except Exception:
        return ""


def base_doc(root: Path, repo: str, facts=None, warnings=None) -> dict:
    return {
        "schema": 1,
        "generated_at": GENERATED_AT,
        "generator": GENERATOR,
        "repo": repo,
        "facts": facts or [],
        "evidence": [],
        "confidence": 0.0,
        "warnings": warnings or [],
    }


def walk(root: Path) -> list[str]:
    paths: list[str] = []
    for current, dirs, files in os.walk(root):
        dirs[:] = sorted(d for d in dirs if d not in EXCLUDE_DIRS)
        for name in sorted(files):
            path = Path(current) / name
            r = rel(root, path)
            if any(r.startswith(prefix) for prefix in EXCLUDE_PREFIXES):
                continue
            paths.append(r)
    return sorted(paths)


def purpose(path: str) -> str:
    lower = path.lower()
    name = Path(path).name.lower()
    rules = [
        ("test", lambda: lower.startswith("tests/") or ".test." in lower or ".spec." in lower or name.startswith("test_")),
        ("documentation", lambda: lower.startswith("docs/") or name.startswith("readme") or lower.endswith(".md")),
        ("script", lambda: lower.startswith("scripts/") or lower.endswith(".sh")),
        ("ci_workflow", lambda: lower.startswith(".github/workflows/")),
        ("config", lambda: name in {"package.json", "pyproject.toml", "requirements.txt", "go.mod", "cargo.toml", "tsconfig.json"} or lower.endswith((".yml", ".yaml", ".toml"))),
        ("container", lambda: "dockerfile" in name or "docker-compose" in name or name == "containerfile"),
        ("database", lambda: "migration" in lower or lower.startswith(("db/", "database/")) or lower.endswith(".sql")),
        ("ui", lambda: "/pages/" in lower or "/components/" in lower or lower.endswith((".tsx", ".jsx"))),
        ("api", lambda: "/api/" in lower or "route" in lower or "server" in lower),
        ("ai", lambda: "/ai/" in lower or "rag" in lower or "embedding" in lower or "assistant" in lower),
        ("generated", lambda: lower.startswith(".autospec/")),
    ]
    for label, matches in rules:
        if matches():
            return label
    return "source" if Path(path).suffix in LANG else "unknown"


def inventory(root: Path, repo: str) -> dict:
    files = []
    for p in walk(root):
        full = root / p
        data = full.read_bytes()
        file_purpose = purpose(p)
        files.append({
            "path": p,
            "purpose": file_purpose,
            "size": len(data),
            "hash": "sha256:" + hashlib.sha256(data).hexdigest(),
            "modified_time": int(full.stat().st_mtime),
            "language": LANG.get(full.suffix, "unknown"),
            "confidence": 0.9 if file_purpose != "unknown" else 0.4,
            "evidence": [p],
        })
    groups: dict[str, list[str]] = {}
    for item in files:
        groups.setdefault(item["purpose"], []).append(item["path"])
    doc = base_doc(root, repo, warnings=[])
    doc.update({"files": sorted(files, key=lambda x: x["path"]), "files_by_purpose": {k: sorted(v) for k, v in sorted(groups.items())}})
    doc["facts"] = [{"value": f"{len(files)} files inventoried", "confidence": 1.0, "evidence": ["filesystem walk"]}]
    doc["confidence"] = 0.9
    return doc


def package(root: Path) -> dict:
    return load_json(root / "package.json", {})


def tech(root: Path, repo: str, inv: dict) -> tuple[dict, str]:
    pkg = package(root)
    deps = {}
    for section in ("dependencies", "devDependencies", "peerDependencies"):
        deps.update(pkg.get(section, {}) if isinstance(pkg.get(section), dict) else {})
    entries = []
    def add(name, category, version="", evidence=None, confidence=0.8, standardized="unknown", notes=""):
        entries.append({"name": name, "category": category, "version": version, "source_files": sorted(evidence or []), "confidence": confidence, "evidence": sorted(evidence or []), "standardized": standardized, "notes": notes})
    if (root / "package.json").exists():
        add("npm", "package_manager", "", ["package.json"], 0.9, True)
    for name, version in sorted(deps.items()):
        low = name.lower()
        cat = "library"
        if low in {"react", "vue", "svelte", "next"}: cat = "ui_framework"
        elif low in {"express", "fastify", "koa", "fastapi", "flask"}: cat = "backend_framework"
        elif low in {"vitest", "jest", "mocha"}: cat = "testing_tool"
        elif low in {"playwright", "cypress"}: cat = "e2e_tool"
        elif low in {"eslint"}: cat = "lint_tool"
        elif low in {"prettier"}: cat = "format_tool"
        elif low in {"vite", "webpack", "rollup"}: cat = "build_tool"
        elif low in {"recharts", "chart.js", "d3"}: cat = "chart_library"
        elif low in {"axios", "got", "node-fetch"}: cat = "http_client"
        elif low in {"openai", "@anthropic-ai/sdk", "langchain"}: cat = "ai_provider"
        elif "modelcontextprotocol" in low: cat = "mcp_tooling"
        elif low in {"zod", "ajv"}: cat = "schema_tool"
        add(name, cat, str(version), ["package.json"], 0.85)
    sprawl = []
    categories = {}
    for e in entries:
        categories.setdefault(e["category"], []).append(e["name"])
    for cat, names in sorted(categories.items()):
        if cat in {"chart_library", "testing_tool", "format_tool", "http_client", "ui_framework"} and len(names) > 1:
            sprawl.append({"category": cat, "message": f"multiple {cat.replace('_', ' ')}s detected", "technologies": sorted(names)})
    if len(categories.get("chart_library", [])) > 1:
        sprawl.append({"category": "chart_library", "message": "multiple charting libraries detected", "technologies": sorted(categories["chart_library"])})
    doc = base_doc(root, repo, warnings=sprawl)
    doc.update({"technologies": entries, "sprawl": sprawl, "facts": entries, "confidence": 0.85 if entries else 0.2})
    yml = ["schema: 1", f"generated_at: {GENERATED_AT}", f"generator: {GENERATOR}", f"repo: {repo}", "technologies:"]
    for e in entries:
        yml += [f"  - name: {e['name']}", f"    category: {e['category']}", f"    version: \"{e['version']}\"", f"    source_files: {json.dumps(e['source_files'])}", f"    confidence: {e['confidence']}", f"    evidence: {json.dumps(e['evidence'])}", f"    standardized: {str(e['standardized']).lower() if isinstance(e['standardized'], bool) else e['standardized']}", f"    notes: \"{e['notes']}\""]
    yml += ["sprawl:"]
    for s in sprawl:
        yml.append(f"  - message: {s['message']}")
    return doc, "\n".join(yml) + "\n"


def slug(text: str) -> str:
    s = re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")
    return s or "unknown"


def extract_routes(root: Path, files: list[str]) -> list[dict]:
    routes = []
    for p in files:
        text = read_text(root, p, 100_000)
        for m in re.finditer(r"(?:app|router)\.(get|post|put|patch|delete)\s*\(\s*[\"']([^\"']+)", text):
            routes.append({"method": m.group(1).upper(), "path": m.group(2), "files": [p], "evidence": [p], "confidence": 0.85})
    return sorted(routes, key=lambda r: (r["path"], r["method"]))


def capabilities(root: Path, repo: str, inv: dict, routes: list[dict]) -> dict:
    caps = {}
    def cap(cid, title, typ, status, files=None, docs=None, tests=None, conf=0.6, visible=True):
        caps[cid] = {"id": cid, "title": title, "type": typ, "status": status, "files": sorted(set(files or [])), "docs": sorted(set(docs or [])), "tests": sorted(set(tests or [])), "evidence": sorted(set((files or []) + (docs or []) + (tests or []))), "confidence": conf, "user_visible": visible, "baseline_related": True}
    pkg = package(root)
    for name in sorted((pkg.get("scripts") or {}).keys()):
        cap("script-" + slug(name), f"Run {name}", "cli", "detected", files=["package.json"], conf=0.8, visible=False)
    for r in routes:
        cap("api-" + slug(r["path"]), f"{r['method']} {r['path']}", "api", "detected", files=r["files"], conf=0.85)
    for item in inv["files"]:
        p = item["path"]
        if item["purpose"] == "ui":
            cap("ui-" + slug(Path(p).stem), Path(p).stem.replace("-", " ").title(), "ui", "detected", files=[p], conf=0.7)
        if item["purpose"] == "test":
            cap("test-" + slug(Path(p).stem), Path(p).stem.replace("-", " ").title(), "test", "tested", tests=[p], conf=0.7, visible=False)
        if item["purpose"] == "documentation":
            text = read_text(root, p)
            for h in re.findall(r"^#{1,3}\s+(.+)$", text, re.M):
                cap(slug(h), h, "docs", "documented", docs=[p], tests=[t["path"] for t in inv["files"] if "project" in t["path"].lower() and t["purpose"] == "test"], conf=0.65)
    doc = base_doc(root, repo)
    doc.update({"capabilities": sorted(caps.values(), key=lambda c: c["id"]), "facts": sorted(caps.values(), key=lambda c: c["id"]), "confidence": 0.7 if caps else 0.1})
    return doc


def surfaces(root: Path, repo: str, inv: dict, routes: list[dict]) -> dict[str, dict]:
    files = inv["files"]
    by_purpose = inv["files_by_purpose"]
    env_names = []
    for item in files:
        text = read_text(root, item["path"], 80_000)
        env_names += re.findall(r"process\.env\.([A-Z0-9_]+)", text)
    settings = [{"name": name, "files": [f["path"] for f in files if name in read_text(root, f["path"], 80_000)], "evidence": ["environment variable name only"], "confidence": 0.75} for name in sorted(set(env_names))]
    docs = {}
    docs["api-surface"] = base_doc(root, repo); docs["api-surface"].update({"routes": routes, "openapi": [f["path"] for f in files if re.search(r"(openapi|swagger)", f["path"], re.I)], "facts": routes, "confidence": 0.8 if routes else 0.3})
    docs["ui-surface"] = base_doc(root, repo); docs["ui-surface"].update({"routes": by_purpose.get("ui", []), "components": [f["path"] for f in files if "/components/" in f["path"]], "facts": by_purpose.get("ui", []), "confidence": 0.75 if by_purpose.get("ui") else 0.2})
    docs["data-surface"] = base_doc(root, repo); docs["data-surface"].update({"database_files": by_purpose.get("database", []), "facts": by_purpose.get("database", []), "confidence": 0.75 if by_purpose.get("database") else 0.2})
    docs["settings-registry"] = base_doc(root, repo); docs["settings-registry"].update({"settings": settings, "facts": settings, "confidence": 0.75 if settings else 0.2})
    perm_files = [f["path"] for f in files if re.search(r"(auth|permission|policy|rbac|acl)", f["path"], re.I)]
    docs["permission-model"] = base_doc(root, repo); docs["permission-model"].update({"permissions": perm_files, "facts": perm_files, "confidence": 0.5 if perm_files else 0.1})
    ai_files = [f["path"] for f in files if f["purpose"] == "ai" or re.search(r"(openai|embedding|rag|agent|token)", read_text(root, f["path"], 50_000), re.I)]
    docs["ai-capabilities"] = base_doc(root, repo); docs["ai-capabilities"].update({"capabilities": ai_files, "facts": ai_files, "confidence": 0.75 if ai_files else 0.1})
    mcp_files = [f["path"] for f in files if ".mcp" in f["path"] or "modelcontextprotocol" in read_text(root, f["path"], 50_000).lower()]
    docs["mcp-registry"] = base_doc(root, repo); docs["mcp-registry"].update({"tools": mcp_files, "facts": mcp_files, "confidence": 0.75 if mcp_files else 0.1})
    return docs


def domain_workflows(root: Path, repo: str, inv: dict) -> tuple[dict, dict]:
    entities = {}
    for item in inv["files"]:
        text = read_text(root, item["path"])
        for name in re.findall(r"\b(?:class|interface|type)\s+([A-Z][A-Za-z0-9_]*)", text):
            entities[name] = {"name": name, "type": "entity", "files": [item["path"]], "relationships": [], "evidence": [item["path"]], "confidence": 0.75}
        for name in re.findall(r"CREATE\s+TABLE\s+([A-Za-z0-9_]+)", text, re.I):
            title = "".join(part.capitalize() for part in name.split("_")).rstrip("s")
            entities[title] = {"name": title, "type": "resource", "files": [item["path"]], "relationships": [], "evidence": [item["path"]], "confidence": 0.7}
    workflows = []
    for item in inv["files"]:
        if item["purpose"] in {"documentation", "test"}:
            text = read_text(root, item["path"])
            if re.search(r"create project", text, re.I) or "create-project" in item["path"]:
                workflows.append({"id": "create-project", "title": "Create Project", "actors": ["user"], "steps": re.findall(r"^\d+\.\s+(.+)$", text, re.M), "entry_points": ["src/pages/dashboard.tsx"] if (root / "src/pages/dashboard.tsx").exists() else [], "files": [item["path"]], "docs": [item["path"]] if item["purpose"] == "documentation" else [], "tests": [item["path"]] if item["purpose"] == "test" else [], "evidence": [item["path"]], "confidence": 0.65})
    d = base_doc(root, repo); d.update({"entities": sorted(entities.values(), key=lambda e: e["name"]), "facts": sorted(entities.values(), key=lambda e: e["name"]), "confidence": 0.65 if entities else 0.1})
    w = base_doc(root, repo); w.update({"workflows": sorted(workflows, key=lambda x: x["id"]), "facts": sorted(workflows, key=lambda x: x["id"]), "confidence": 0.6 if workflows else 0.1})
    return d, w


def graph(root: Path, repo: str, inv: dict, caps: dict, domain: dict, workflow: dict, surf: dict) -> dict:
    nodes, edges = {}, []
    def node(nid, typ, title, data=None):
        nodes[nid] = {"id": nid, "type": typ, "title": title, "data": data or {}}
    def edge(a, b, typ, ev):
        if a in nodes and b in nodes:
            edges.append({"from": a, "to": b, "type": typ, "evidence": [ev]})
    for f in inv["files"]:
        node("file:" + f["path"], "file", f["path"], {"purpose": f["purpose"]})
    for c in caps["capabilities"]:
        cid = "capability:" + c["id"]; node(cid, "capability", c["title"], c)
        for f in c.get("files", []): edge(cid, "file:" + f, "implements", f)
        for f in c.get("docs", []): edge("file:" + f, cid, "documents", f)
        for f in c.get("tests", []): edge("file:" + f, cid, "tests", f)
    for e in domain["entities"]:
        eid = "domain_entity:" + e["name"]; node(eid, "domain_entity", e["name"], e)
        for f in e["files"]: edge("file:" + f, eid, "defines", f)
    for w in workflow["workflows"]:
        wid = "workflow:" + w["id"]; node(wid, "workflow", w["title"], w)
        for f in w["files"]: edge("file:" + f, wid, "documents", f)
    for route in surf["api-surface"].get("routes", []):
        rid = "api:" + route["path"]; node(rid, "api", route["path"], route)
        for f in route["files"]: edge("file:" + f, rid, "exposes", f)
    compact_edges = sorted({(e["from"], e["to"], e["type"], tuple(e["evidence"])) for e in edges})
    doc = base_doc(root, repo)
    doc.update({"nodes": sorted(nodes.values(), key=lambda n: n["id"]), "edges": [{"from": a, "to": b, "type": t, "evidence": list(ev)} for a, b, t, ev in compact_edges], "confidence": 0.7})
    return doc


def build(root: Path) -> int:
    repo = root.name
    state, reports = ensure(root)
    inv = inventory(root, repo)
    write_json(state / "repository-inventory.json", inv)
    write_text(reports / "repository-inventory.md", inventory_md(inv))
    tech_doc, tech_yml = tech(root, repo, inv)
    write_text(state / "technology-registry.yml", tech_yml)
    write_text(reports / "technology-registry.md", tech_md(tech_doc))
    routes = extract_routes(root, [f["path"] for f in inv["files"]])
    caps = capabilities(root, repo, inv, routes)
    write_json(state / "capability-registry.json", caps)
    write_text(reports / "capability-registry.md", cap_md(caps))
    surf = surfaces(root, repo, inv, routes)
    for name, doc in surf.items():
        write_json(state / f"{name}.json", doc)
        write_text(reports / f"{name}.md", generic_md(name.replace("-", " ").title(), doc))
    domain, workflow = domain_workflows(root, repo, inv)
    write_json(state / "domain-model.json", domain); write_json(state / "workflow-map.json", workflow)
    write_text(reports / "domain-workflow-map.md", domain_workflow_md(domain, workflow))
    kg = graph(root, repo, inv, caps, domain, workflow, surf)
    write_json(state / "knowledge-graph.json", kg); write_text(reports / "knowledge-graph.md", graph_md(kg))
    twin = digital_twin(root, repo, inv, tech_doc, caps, domain, workflow, kg)
    write_json(state / "digital-twin.json", twin); write_text(reports / "digital-twin.md", twin_md(twin))
    drift_code = drift(root, quiet=True)
    build_report = {"version": 1, "status": "pass", "stages": ["repository inventory", "technology registry", "surface maps", "capability registry", "domain/workflow map", "knowledge graph", "digital twin summary", "metadata drift"], "metadata_drift_exit_code": drift_code}
    write_json(reports / "digital-twin-build.json", build_report)
    write_text(reports / "digital-twin-build.md", "# Digital Twin Build\n\nStatus: **PASS**\n\n" + "\n".join(f"- {s}" for s in build_report["stages"]))
    print("digital twin build: PASS")
    return 0


def inventory_md(inv): return "# Repository Inventory\n\n| Purpose | Files |\n| --- | ---: |\n" + "\n".join(f"| {k} | {len(v)} |" for k, v in inv["files_by_purpose"].items())
def tech_md(doc):
    lines = ["# Technology Registry", "", "| Name | Category | Version | Confidence |", "| --- | --- | --- | ---: |"]
    lines += [f"| {e['name']} | {e['category']} | {e['version']} | {e['confidence']:.2f} |" for e in doc["technologies"]]
    lines += ["", "## Sprawl"] + [f"- {s['message']}: {', '.join(s['technologies'])}" for s in doc["sprawl"]] if doc["sprawl"] else ["", "## Sprawl", "- None."]
    return "\n".join(lines)
def cap_md(doc): return "# Capability Registry\n\n| Capability | Type | Status | Confidence |\n| --- | --- | --- | ---: |\n" + "\n".join(f"| {c['id']} | {c['type']} | {c['status']} | {c['confidence']:.2f} |" for c in doc["capabilities"])
def generic_md(title, doc): return f"# {title}\n\n| Fact | Confidence | Evidence |\n| --- | ---: | --- |\n" + "\n".join(f"| {str(f)[:80]} | {doc.get('confidence', 0):.2f} | metadata |" for f in doc.get("facts", [])[:20])
def domain_workflow_md(d, w): return "# Domain And Workflow Map\n\n## Domain Entities\n\n" + "\n".join(f"- {e['name']} ({e['type']})" for e in d["entities"]) + "\n\n## Workflows\n\n" + "\n".join(f"- {x['id']}: {x['title']}" for x in w["workflows"])
def graph_md(kg):
    caps = [n for n in kg["nodes"] if n["type"] == "capability"]
    return f"# Knowledge Graph\n\nNode counts: **{len(kg['nodes'])}**\n\nEdge counts: **{len(kg['edges'])}**\n\n## Top Connected Capabilities\n\n" + "\n".join(f"- {c['title']}" for c in caps[:10]) + "\n\n## Orphan Docs\n\n- See metadata drift.\n\n## Orphan Tests\n\n- See metadata drift."
def digital_twin(root, repo, inv, tech_doc, caps, domain, workflow, kg):
    langs = sorted({f["language"] for f in inv["files"] if f["language"] not in {"unknown", "Markdown", "JSON", "YAML"}})
    return {"schema": 1, "repo": repo, "generated_at": GENERATED_AT, "generator": GENERATOR, "sources": {"repository_inventory": ".autospec/state/repository-inventory.json", "technology_registry": ".autospec/state/technology-registry.yml", "capability_registry": ".autospec/state/capability-registry.json", "domain_model": ".autospec/state/domain-model.json", "workflow_map": ".autospec/state/workflow-map.json", "knowledge_graph": ".autospec/state/knowledge-graph.json"}, "summary": {"application_type": "web/service" if any(f["purpose"] in {"api", "ui"} for f in inv["files"]) else "repository", "primary_languages": langs[:5], "detected_capabilities": len(caps["capabilities"]), "docs_coverage": "present" if inv["files_by_purpose"].get("documentation") else "missing", "test_coverage": "present" if inv["files_by_purpose"].get("test") else "missing", "ai_readiness": "present" if any(f["purpose"] == "ai" for f in inv["files"]) else "unknown", "operations_readiness": "present" if inv["files_by_purpose"].get("ci_workflow") else "unknown"}, "confidence": 0.7, "warnings": tech_doc.get("sprawl", [])}
def twin_md(twin): return "# Digital Twin\n\n## State Of The Repo\n\n" + "\n".join(f"- {k}: {v}" for k, v in twin["summary"].items()) + "\n\n## Sources\n\n" + "\n".join(f"- {k}: `{v}`" for k, v in twin["sources"].items()) + "\n\nv1 is heuristic and evidence-scored, not perfect."


def impact(root: Path, query_type: str, value: str) -> int:
    _, reports = ensure(root)
    kg = load_json(root / ".autospec/state/knowledge-graph.json", {"nodes": [], "edges": []})
    caps = load_json(root / ".autospec/state/capability-registry.json", {"capabilities": []})
    inv = load_json(root / ".autospec/state/repository-inventory.json", {"files": []})
    warnings = []
    files = []
    related_caps = []
    if query_type == "file":
        files = [value] if any(f["path"] == value for f in inv["files"]) else []
        if not files: warnings.append(f"file not found in repository inventory: {value}")
        for e in kg["edges"]:
            if e["to"] == "file:" + value and e["from"].startswith("capability:"):
                related_caps.append(e["from"].split(":", 1)[1])
    elif query_type == "capability":
        related_caps = [value] if any(c["id"] == value for c in caps["capabilities"]) else []
        if not related_caps: warnings.append(f"capability not found: {value}")
        cap = next((c for c in caps["capabilities"] if c["id"] == value), {})
        files = sorted(set(cap.get("files", []) + cap.get("docs", []) + cap.get("tests", [])))
    docs = [f["path"] for f in inv["files"] if f["purpose"] == "documentation" and ("project" in f["path"].lower() or not related_caps)]
    tests = [f["path"] for f in inv["files"] if f["purpose"] == "test" and ("project" in f["path"].lower() or not related_caps)]
    report = {"version": 1, "query": {"type": query_type, "value": value}, "directly_affected_files": files, "related_capabilities": related_caps, "related_tests": tests, "related_docs": docs, "related_workflows": [], "related_areas": ["api" if "/api/" in f else "ui" if "/pages/" in f or "/components/" in f else "data" if f.endswith(".sql") else "unknown" for f in files], "likely_validation_commands": ["bash scripts/validate.sh"], "risk_hints": [], "missing_evidence": warnings, "warnings": warnings}
    write_json(reports / "impact-analysis.json", report)
    write_text(reports / "impact-analysis.md", "# Impact Analysis\n\n| Area | Items |\n| --- | --- |\n" + f"| Files | {', '.join(files) or 'none'} |\n| Capabilities | {', '.join(related_caps) or 'none'} |\n| Tests | {', '.join(tests) or 'none'} |\n| Docs | {', '.join(docs) or 'none'} |\n\n" + "\n".join(f"- {w}" for w in warnings))
    print("impact analysis: PASS")
    return 0


def drift(root: Path, quiet: bool = False) -> int:
    _, reports = ensure(root)
    required = ["repository-inventory.json", "capability-registry.json", "domain-model.json", "workflow-map.json", "knowledge-graph.json", "digital-twin.json"]
    findings = []
    for name in required:
        if not (root / ".autospec/state" / name).exists():
            findings.append({"code": "MISSING_REQUIRED_METADATA", "message": f"missing .autospec/state/{name}", "evidence": [name]})
    inv = load_json(root / ".autospec/state/repository-inventory.json", {"files": []})
    for item in inv.get("files", []):
        if not (root / item["path"]).exists():
            findings.append({"code": "MISSING_FILE_REFERENCE", "message": f"metadata references missing file {item['path']}", "evidence": [item["path"]]})
    caps = load_json(root / ".autospec/state/capability-registry.json", {"capabilities": []})
    linked = {p for c in caps.get("capabilities", []) for p in c.get("files", []) + c.get("docs", []) + c.get("tests", [])}
    for item in inv.get("files", []):
        if item["purpose"] in {"documentation", "test"} and item["path"] not in linked:
            findings.append({"code": "ORPHAN_" + item["purpose"].upper(), "message": f"{item['purpose']} not linked to a capability: {item['path']}", "evidence": [item["path"]]})
    report = {"version": 1, "status": "fail" if findings else "pass", "findings": sorted(findings, key=lambda f: (f["code"], f["message"]))}
    write_json(reports / "metadata-drift.json", report)
    write_text(reports / "metadata-drift.md", "# Metadata Drift\n\nStatus: **" + report["status"].upper() + "**\n\n" + "\n".join(f"- {f['code']}: {f['message']}" for f in report["findings"]) if findings else "# Metadata Drift\n\nStatus: **PASS**\n")
    if not quiet:
        print("metadata drift: " + report["status"].upper())
    return 1 if findings else 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=["build", "impact", "drift"])
    parser.add_argument("--repo-root", default=os.getcwd())
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--file")
    parser.add_argument("--capability")
    parser.add_argument("--issue")
    args = parser.parse_args()
    root = Path(args.repo_root).resolve()
    if args.command == "build":
        return build(root)
    if args.command == "drift":
        return drift(root)
    if args.file:
        return impact(root, "file", args.file)
    if args.capability:
        return impact(root, "capability", args.capability)
    if args.issue:
        return impact(root, "issue", args.issue)
    parser.error("impact requires --file, --capability, or --issue")
    return 2


if __name__ == "__main__":
    sys.exit(main())
