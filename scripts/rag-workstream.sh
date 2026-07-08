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
USAGE
}

if [ "${1:-}" = "--help" ] || [ $# -eq 0 ]; then
    usage
    exit 0
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
REQUIRED_RETRIEVAL_KNOBS = ["dense_top_k", "bm25_top_k", "rrf_k", "rerank_top_n", "final_n"]


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



def stable_rel(path, root):
    return path.resolve().relative_to(root.resolve()).as_posix()


def file_hash_text(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def chunk_config(config):
    chunking = dict(config.get("chunking", {}))
    return {
        "strategy": chunking.get("strategy", "structure_aware"),
        "chunk_size": int(chunking.get("chunk_size", 900)),
        "overlap": int(chunking.get("overlap", 0)),
        "late_chunking": bool(chunking.get("late_chunking", False)),
        "contextual_prefix": bool(chunking.get("contextual_prefix", False)),
        "preserve_code_fences": bool(chunking.get("preserve_code_fences", True)),
        "preserve_heading_body": bool(chunking.get("preserve_heading_body", True)),
    }


def chunk_config_hash(config):
    return hashlib.sha256(canonical(chunk_config(config)).encode("utf-8")).hexdigest()[:16]


def iter_corpus_files(root, config):
    corpus = config.get("corpus", {})
    includes = corpus.get("include", ["llms.txt", "llms-full.txt", "docs/**/*.md"])
    excludes = corpus.get("exclude", [])
    paths = []
    for pattern in includes:
        if any(ch in pattern for ch in "*?["):
            paths.extend(root.glob(pattern))
        else:
            candidate = root / pattern
            if candidate.exists():
                paths.append(candidate)
    unique = []
    seen = set()
    for path in sorted(paths, key=lambda x: stable_rel(x, root)):
        if not path.is_file():
            continue
        rel = stable_rel(path, root)
        if rel in seen:
            continue
        if any(Path(rel).match(pattern) for pattern in excludes):
            continue
        seen.add(rel)
        unique.append(path)
    return unique


def heading_title(line):
    m = re.match(r"^(#{1,6})\s+(.+?)\s*$", line)
    if not m:
        return ""
    return m.group(2).strip()


def markdown_sections(text):
    lines = text.splitlines()
    sections = []
    current = []
    current_heading = "Document"
    in_fence = False
    for line in lines:
        if line.startswith("```"):
            in_fence = not in_fence
        if not in_fence and re.match(r"^#{1,6}\s+", line) and current:
            sections.append((current_heading, "\n".join(current).strip() + "\n"))
            current = []
        if not in_fence and re.match(r"^#{1,6}\s+", line):
            current_heading = heading_title(line)
        current.append(line)
    if current:
        sections.append((current_heading, "\n".join(current).strip() + "\n"))
    return sections


def section_blocks(section_text):
    blocks = []
    current = []
    in_fence = False
    for line in section_text.splitlines():
        if line.startswith("```"):
            in_fence = not in_fence
            current.append(line)
            if not in_fence:
                blocks.append("\n".join(current).strip() + "\n")
                current = []
            continue
        if in_fence:
            current.append(line)
            continue
        if not line.strip():
            if current:
                blocks.append("\n".join(current).strip() + "\n")
                current = []
            continue
        current.append(line)
    if current:
        blocks.append("\n".join(current).strip() + "\n")
    return blocks


def split_section(heading, section_text, max_chars, overlap):
    # Keep the markdown heading attached to every emitted slice so no chunk is a
    # bare heading detached from body text. Code-fence blocks are never split.
    blocks = section_blocks(section_text)
    if not blocks:
        return []
    heading_block = blocks[0] if re.match(r"^#{1,6}\s+", blocks[0]) else ""
    body_blocks = blocks[1:] if heading_block else blocks
    if not body_blocks:
        return [section_text]
    chunks = []
    current = heading_block
    previous_tail = ""
    for block in body_blocks:
        candidate = current + block
        if len(candidate) > max_chars and current.strip() != heading_block.strip():
            chunks.append(current)
            tail = current[-overlap:] if overlap > 0 else ""
            previous_tail = tail.lstrip()
            current = heading_block
            if previous_tail and previous_tail not in current:
                current += previous_tail + "\n"
        current += block
    if current.strip():
        chunks.append(current)
    return chunks


def make_contextual_prefix(rel, heading, ordinal):
    location = f"Document: {rel}"
    if heading and heading != "Document":
        location += f" > {heading}"
    return f"{location}. Chunk {ordinal} context: use this passage as part of the autospec documentation corpus."


def make_chunk(rel, doc_hash, text, heading, ordinal, cfg, full_doc_text):
    prefix = make_contextual_prefix(rel, heading, ordinal) if cfg["contextual_prefix"] else ""
    embedding_basis = full_doc_text if cfg["late_chunking"] else ((prefix + "\n") if prefix else "") + text
    chunk_id = hashlib.sha256(f"{rel}\0{ordinal}\0{text}".encode("utf-8")).hexdigest()[:16]
    record = {
        "chunk_id": chunk_id,
        "doc_id": rel,
        "source_path": rel,
        "doc_content_hash": doc_hash,
        "ordinal": ordinal,
        "heading": heading,
        "text": text,
        "text_hash": file_hash_text(text),
        "embedding_input_hash": file_hash_text(embedding_basis),
        "strategy": cfg["strategy"],
        "late_chunking": cfg["late_chunking"],
    }
    if prefix:
        record["contextual_prefix"] = prefix
    return record


def validate_ingest_config(config):
    missing = [section for section in REQUIRED_CONFIG_SECTIONS if section not in config]
    if missing:
        return None, "CONFIG_SECTION_MISSING:" + ",".join(missing)
    cfg = chunk_config(config)
    if cfg["strategy"] != "structure_aware":
        return None, f"UNSUPPORTED_CHUNK_STRATEGY:{cfg['strategy']}"
    if cfg["chunk_size"] <= 0:
        return None, "CHUNK_SIZE_INVALID:must_be_positive"
    if cfg["overlap"] < 0 or cfg["overlap"] >= cfg["chunk_size"]:
        return None, "CHUNK_OVERLAP_INVALID:must_be_nonnegative_and_less_than_chunk_size"
    return cfg, None


def chunk_markdown_doc(path, root, cfg, first_ordinal):
    rel = stable_rel(path, root)
    text = path.read_text(encoding="utf-8")
    doc_hash = file_hash_text(text)
    pairs = []
    for heading, section_text in markdown_sections(text):
        parts = split_section(heading, section_text, cfg["chunk_size"], cfg["overlap"])
        for part in parts:
            nonblank = [line.strip() for line in part.splitlines() if line.strip()]
            if heading and len(nonblank) <= 1:
                continue
            pairs.append((heading, part))
    chunks = [make_chunk(rel, doc_hash, part, heading, first_ordinal + i, cfg, text)
              for i, (heading, part) in enumerate(pairs)]
    doc = {"doc_id": rel, "path": rel, "content_hash": doc_hash, "chunk_count": len(chunks)}
    return doc, chunks


def build_local_index(config, root):
    cfg_hash = chunk_config_hash(config)
    cfg = chunk_config(config)
    model_id = str(config.get("embedding", {}).get("model_id", "local-deterministic"))
    prefix = config.get("embedding", {}).get("index_version_prefix", "rag-index-v1")
    docs, chunks = [], []
    for path in iter_corpus_files(root, config):
        doc, new_chunks = chunk_markdown_doc(path, root, cfg, len(chunks) + 1)
        docs.append(doc)
        chunks.extend(new_chunks)
    return {
        "schema_version": 1,
        "index_version": f"{prefix}:{model_id}:{cfg_hash}",
        "embedding_model_id": model_id,
        "chunking": {**cfg, "chunk_config_hash": cfg_hash},
        "index_metadata": {
            "reembed_policy": "clean_reembed_on_model_or_chunk_config_change",
            "version_tuple": {"embedding_model_id": model_id, "chunk_config_hash": cfg_hash},
            "generated_by": "scripts/rag-workstream.sh ingest-index",
        },
        "docs": docs,
        "chunks": chunks,
    }


def cmd_ingest_index(args):
    config = load_json(args.config)
    cfg, error = validate_ingest_config(config)
    if error:
        print(error)
        return 1
    out = Path(args.out)
    index = build_local_index(config, Path(args.root))
    write_json(out, index)
    print(f"rag ingest-index wrote {len(index['chunks'])} chunks from {len(index['docs'])} docs to {out}")
    return 0

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
    p = sub.add_parser("ingest-index")
    p.add_argument("--config", required=True)
    p.add_argument("--root", required=True)
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_ingest_index)
    return parser


args = build_parser().parse_args(sys.argv[1:])
raise SystemExit(args.func(args))
PY
