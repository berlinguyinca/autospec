#!/usr/bin/env bash
# scripts/rag-workstream.sh — deterministic foundation gate for issue #1541 RAG documentation database.
# Provides versioned config hashing, eval-promotion gates, freshness/citation/chunk
# guards, and design-doc source validation without adding runtime dependencies.

set -eu

usage() {
    cat <<'USAGE'
Usage:
  rag-workstream.sh config-version --config FILE
  rag-workstream.sh gate --baseline FILE --candidate FILE [--target-metric ndcg] [--faithfulness-floor N] [--promote-out FILE]
  rag-workstream.sh freshness-check --manifest FILE --root DIR
  rag-workstream.sh chunk-boundary-check --chunks FILE
  rag-workstream.sh citation-check --claims FILE --chunks FILE
  rag-workstream.sh validate-design-doc --doc FILE
  rag-workstream.sh ingest-index --config FILE --root DIR --out FILE
  rag-workstream.sh retrieve --index FILE --config FILE --query TEXT [--filter KEY=VALUE] [knobs]
  rag-workstream.sh retrieve-eval --index FILE --config FILE --golden FILE [--mode hybrid|dense|bm25] [knobs]
  rag-workstream.sh query-router-eval --index FILE --config FILE --golden FILE [--mode hybrid|dense|bm25] [knobs]
USAGE
}

if [ "${1:-}" = "--help" ] || [ $# -eq 0 ]; then
    usage
    exit 0
fi

if [ "${1:-}" = "ingest-index" ]; then
    shift
    SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
    python3 "$SCRIPT_DIR/rag-ingest-index.py" "$@"
    exit $?
fi

if [ "${1:-}" = "retrieve" ] || [ "${1:-}" = "retrieve-eval" ] || [ "${1:-}" = "query-router-eval" ]; then
    CMD="$1"
    shift
    SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
    python3 "$SCRIPT_DIR/rag_retrieve.py" "$CMD" "$@"
    exit $?
fi

python3 - "$@" <<'PY'
import argparse
import hashlib
import json
import os
import re
import sys
from pathlib import Path

SOURCE_TOKENS = [
    "Anthropic Contextual Retrieval",
    "https://www.anthropic.com/engineering/contextual-retrieval",
    "RAGAS",
    "https://docs.ragas.io/en/stable/concepts/metrics/available_metrics/faithfulness/",
    "Jina late-chunking",
    "https://jina.ai/news/late-chunking-in-long-context-embedding-models/",
    "llms.txt",
    "https://llmstxt.org/",
]
CHILD_TOKENS = ["#1548", "#1549", "#1550", "#1551", "#1552"]
REQUIRED_CONFIG_SECTIONS = ["corpus", "chunking", "embedding", "retrieval", "query_transform", "freshness", "eval"]
REQUIRED_RETRIEVAL_KNOBS = ["dense_top_k", "bm25_top_k", "rrf_k", "fusion_weight", "rerank_top_n", "final_n"]


def load_json(path):
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def write_json(path, obj):
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(obj, fh, indent=2, sort_keys=True)
        fh.write("\n")


def canonical(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def nonneg_float(value, flag):
    try:
        parsed = float(value)
    except (TypeError, ValueError):
        raise SystemExit(f"{flag} must be a number")
    if parsed < 0:
        raise SystemExit(f"{flag} must be non-negative")
    return parsed


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for block in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def chunks_by_id(chunks_doc):
    chunks = chunks_doc.get("chunks", chunks_doc if isinstance(chunks_doc, list) else [])
    return {str(chunk.get("chunk_id")): chunk for chunk in chunks}



def cmd_config_version(args):
    config = load_json(args.config)
    missing = [section for section in REQUIRED_CONFIG_SECTIONS if section not in config]
    if missing:
        print("CONFIG_SECTION_MISSING:" + ",".join(missing))
        return 1
    retrieval = config.get("retrieval", {})
    missing_knobs = [knob for knob in REQUIRED_RETRIEVAL_KNOBS if knob not in retrieval]
    if missing_knobs:
        print("CONFIG_KNOB_MISSING:" + ",".join(missing_knobs))
        return 1
    prefix = config.get("embedding", {}).get("index_version_prefix", "rag-index-v1")
    digest = hashlib.sha256(canonical(config).encode("utf-8")).hexdigest()[:16]
    print(f"{prefix}:{digest}")
    return 0


def metric(doc, layer, metric):
    try:
        return float(doc[layer][metric])
    except KeyError:
        raise SystemExit(f"score missing: {layer}.{metric}")


def cmd_gate(args):
    baseline = load_json(args.baseline)
    candidate = load_json(args.candidate)
    target = args.target_metric
    floor = nonneg_float(args.faithfulness_floor, "--faithfulness-floor")
    base_target = metric(baseline, "retrieval", target)
    cand_target = metric(candidate, "retrieval", target)
    base_faith = metric(baseline, "ragas", "faithfulness")
    cand_faith = metric(candidate, "ragas", "faithfulness")
    findings = []
    if cand_target <= base_target:
        findings.append(f"TARGET_METRIC_NOT_IMPROVED:{target}:{cand_target:.3f}<={base_target:.3f}")
    if cand_faith < floor:
        findings.append(f"RAGAS_FAITHFULNESS_FLOOR:{cand_faith:.3f}<{floor:.3f}")
    if cand_faith < base_faith:
        findings.append(f"RAGAS_FAITHFULNESS_REGRESSION:{cand_faith:.3f}<{base_faith:.3f}")
    for ragas_metric in ["answer_relevancy", "context_precision"]:
        if ragas_metric in baseline.get("ragas", {}) and ragas_metric in candidate.get("ragas", {}):
            if float(candidate["ragas"][ragas_metric]) < float(baseline["ragas"][ragas_metric]):
                findings.append(
                    f"RAGAS_{ragas_metric.upper()}_REGRESSION:"
                    f"{float(candidate['ragas'][ragas_metric]):.3f}<{float(baseline['ragas'][ragas_metric]):.3f}"
                )
    if findings:
        for finding in findings:
            print(finding)
        print("rag eval gate failed")
        return 1
    promoted = {
        "promoted_config_version": candidate.get("config_version", "unknown"),
        "baseline_config_version": baseline.get("config_version", "unknown"),
        "target_metric": target,
        "baseline_score": base_target,
        "candidate_score": cand_target,
        "ragas_faithfulness": cand_faith,
        "faithfulness_floor": floor,
    }
    if args.promote_out:
        write_json(args.promote_out, promoted)
    print(
        f"rag eval gate passed: {target} {base_target:.3f}->{cand_target:.3f}; "
        f"faithfulness {cand_faith:.3f}>={floor:.3f}"
    )
    return 0


def cmd_freshness_check(args):
    manifest = load_json(args.manifest)
    root = Path(args.root)
    findings = []
    for doc in manifest.get("docs", []):
        doc_id = str(doc.get("doc_id", "unknown"))
        rel = doc.get("path")
        expected = str(doc.get("content_hash", ""))
        if not rel:
            findings.append(f"DOC_PATH_MISSING:{doc_id}")
            continue
        path = root / rel
        if not path.exists():
            findings.append(f"DOC_MISSING:{doc_id}:{rel}")
            continue
        actual = sha256_file(path)
        if expected != actual:
            findings.append(f"STALE_INDEX:{doc_id}:{expected}->{actual}")
    if findings:
        for finding in findings:
            print(finding)
        print("rag freshness check failed")
        return 1
    print(f"rag freshness check passed ({len(manifest.get('docs', []))} docs)")
    return 0


def cmd_chunk_boundary_check(args):
    doc = load_json(args.chunks)
    chunks = doc.get("chunks", doc if isinstance(doc, list) else [])
    findings = []
    for chunk in chunks:
        chunk_id = str(chunk.get("chunk_id", "unknown"))
        heading = str(chunk.get("heading", "")).strip()
        text = str(chunk.get("text", ""))
        nonblank = [line.strip() for line in text.splitlines() if line.strip()]
        if heading and len(nonblank) <= 1:
            findings.append(f"CHUNK_BOUNDARY_LOSS:{chunk_id}:heading_without_body")
        if text.count("```") % 2:
            findings.append(f"CHUNK_BOUNDARY_LOSS:{chunk_id}:unbalanced_code_fence")
    if findings:
        for finding in findings:
            print(finding)
        print("rag chunk-boundary check failed")
        return 1
    print(f"rag chunk-boundary check passed ({len(chunks)} chunks)")
    return 0


def cmd_citation_check(args):
    claims_doc = load_json(args.claims)
    chunks = chunks_by_id(load_json(args.chunks))
    claims = claims_doc.get("claims", claims_doc if isinstance(claims_doc, list) else [])
    findings = []
    for idx, claim in enumerate(claims, 1):
        chunk_id = str(claim.get("chunk_id", ""))
        span = str(claim.get("supporting_span", ""))
        if not chunk_id:
            findings.append(f"CITATION_MISSING:claim{idx}")
            continue
        chunk = chunks.get(chunk_id)
        if not chunk:
            findings.append(f"CITED_CHUNK_MISSING:{chunk_id}")
            continue
        if not span:
            findings.append(f"SUPPORTING_SPAN_MISSING:{chunk_id}")
            continue
        if span.lower() not in str(chunk.get("text", "")).lower():
            findings.append(f"UNSUPPORTED_CITATION:{chunk_id}:{span}")
    if findings:
        for finding in findings:
            print(finding)
        print("rag citation check failed")
        return 1
    print(f"rag citation check passed ({len(claims)} claims)")
    return 0


def cmd_validate_design_doc(args):
    text = Path(args.doc).read_text(encoding="utf-8")
    findings = []
    for token in SOURCE_TOKENS:
        if token not in text:
            findings.append(f"DESIGN_SOURCE_MISSING:{token}")
    for token in CHILD_TOKENS:
        if token not in text:
            findings.append(f"CHILD_HANDOFF_MISSING:{token}")
    required_phrases = [
        "structure-aware",
        "late chunking",
        "contextual prefix",
        "hybrid dense + BM25",
        "Reciprocal Rank Fusion",
        "RAGAS faithfulness floor",
        "content-hash invalidation",
        "citation verification",
        "promote only score-positive",
        "llms.txt",
    ]
    lower = text.lower()
    for phrase in required_phrases:
        if phrase.lower() not in lower:
            findings.append(f"DESIGN_CONTRACT_MISSING:{phrase}")
    if findings:
        for finding in findings:
            print(finding)
        print("rag design doc validation failed")
        return 1
    print("rag design doc validated")
    return 0


def build_parser():
    parser = argparse.ArgumentParser(prog="rag-workstream.sh")
    sub = parser.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("config-version")
    p.add_argument("--config", required=True)
    p.set_defaults(func=cmd_config_version)
    p = sub.add_parser("gate")
    p.add_argument("--baseline", required=True)
    p.add_argument("--candidate", required=True)
    p.add_argument("--target-metric", default="ndcg")
    p.add_argument("--faithfulness-floor", default="0.90")
    p.add_argument("--promote-out")
    p.set_defaults(func=cmd_gate)
    p = sub.add_parser("freshness-check")
    p.add_argument("--manifest", required=True)
    p.add_argument("--root", required=True)
    p.set_defaults(func=cmd_freshness_check)
    p = sub.add_parser("chunk-boundary-check")
    p.add_argument("--chunks", required=True)
    p.set_defaults(func=cmd_chunk_boundary_check)
    p = sub.add_parser("citation-check")
    p.add_argument("--claims", required=True)
    p.add_argument("--chunks", required=True)
    p.set_defaults(func=cmd_citation_check)
    p = sub.add_parser("validate-design-doc")
    p.add_argument("--doc", required=True)
    p.set_defaults(func=cmd_validate_design_doc)
    return parser


args = build_parser().parse_args(sys.argv[1:])
raise SystemExit(args.func(args))
PY
