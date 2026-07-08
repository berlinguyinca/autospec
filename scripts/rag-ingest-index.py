#!/usr/bin/env python3
"""Deterministic local RAG ingestion/index builder for issue #1548."""
import argparse
import hashlib
import json
import re
from pathlib import Path

REQUIRED_CONFIG_SECTIONS = ["corpus", "chunking", "embedding", "retrieval", "query_transform", "freshness", "eval"]


def canonical(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def write_json(path, obj):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as fh:
        json.dump(obj, fh, indent=2, sort_keys=True)
        fh.write("\n")


def text_hash(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def rel_path(path, root):
    return path.resolve().relative_to(root.resolve()).as_posix()


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


def validate_config(config):
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


def corpus_files(root, config):
    corpus = config.get("corpus", {})
    includes = corpus.get("include", ["llms.txt", "llms-full.txt", "docs/**/*.md"])
    excludes = corpus.get("exclude", [])
    candidates = []
    for pattern in includes:
        candidates.extend(root.glob(pattern) if any(ch in pattern for ch in "*?[") else [root / pattern])
    seen, files = set(), []
    for path in sorted(candidates, key=lambda item: rel_path(item, root) if item.exists() else str(item)):
        if not path.is_file():
            continue
        rel = rel_path(path, root)
        if rel in seen or any(Path(rel).match(pattern) for pattern in excludes):
            continue
        seen.add(rel)
        files.append(path)
    return files


def heading_title(line):
    match = re.match(r"^(#{1,6})\s+(.+?)\s*$", line)
    return match.group(2).strip() if match else ""


def markdown_sections(text):
    sections, current, current_heading, in_fence = [], [], "Document", False
    for line in text.splitlines():
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
    blocks, current, in_fence = [], [], False
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
        elif line.strip():
            current.append(line)
        elif current:
            blocks.append("\n".join(current).strip() + "\n")
            current = []
    if current:
        blocks.append("\n".join(current).strip() + "\n")
    return blocks


def split_section(section_text, max_chars, overlap):
    blocks = section_blocks(section_text)
    if not blocks:
        return []
    heading = blocks[0] if re.match(r"^#{1,6}\s+", blocks[0]) else ""
    body = blocks[1:] if heading else blocks
    if not body:
        return [section_text]
    chunks, current = [], heading
    for block in body:
        if len(current + block) > max_chars and current.strip() != heading.strip():
            chunks.append(current)
            tail = current[-overlap:].lstrip() if overlap > 0 else ""
            current = heading + ((tail + "\n") if tail and tail not in heading else "")
        current += block
    return chunks + ([current] if current.strip() else [])


def contextual_prefix(rel, heading, ordinal):
    location = f"Document: {rel}" + (f" > {heading}" if heading and heading != "Document" else "")
    return f"{location}. Chunk {ordinal} context: use this passage as part of the autospec documentation corpus."


def make_chunk(rel, doc_hash, text, heading, ordinal, cfg, full_text):
    prefix = contextual_prefix(rel, heading, ordinal) if cfg["contextual_prefix"] else ""
    basis = full_text if cfg["late_chunking"] else ((prefix + "\n") if prefix else "") + text
    record = {
        "chunk_id": hashlib.sha256(f"{rel}\0{ordinal}\0{text}".encode("utf-8")).hexdigest()[:16],
        "doc_id": rel,
        "source_path": rel,
        "doc_content_hash": doc_hash,
        "ordinal": ordinal,
        "heading": heading,
        "text": text,
        "text_hash": text_hash(text),
        "embedding_input_hash": text_hash(basis),
        "strategy": cfg["strategy"],
        "late_chunking": cfg["late_chunking"],
    }
    if prefix:
        record["contextual_prefix"] = prefix
    return record


def chunk_doc(path, root, cfg, first_ordinal):
    rel, text = rel_path(path, root), path.read_text(encoding="utf-8")
    doc_hash, pairs = text_hash(text), []
    for heading, section_text in markdown_sections(text):
        for part in split_section(section_text, cfg["chunk_size"], cfg["overlap"]):
            nonblank = [line.strip() for line in part.splitlines() if line.strip()]
            if not (heading and len(nonblank) <= 1):
                pairs.append((heading, part))
    chunks = [make_chunk(rel, doc_hash, part, heading, first_ordinal + i, cfg, text)
              for i, (heading, part) in enumerate(pairs)]
    return {"doc_id": rel, "path": rel, "content_hash": doc_hash, "chunk_count": len(chunks)}, chunks


def build_index(config, root, cfg):
    model = str(config.get("embedding", {}).get("model_id", "local-deterministic"))
    prefix = config.get("embedding", {}).get("index_version_prefix", "rag-index-v1")
    cfg_hash = hashlib.sha256(canonical(cfg).encode("utf-8")).hexdigest()[:16]
    docs, chunks = [], []
    for path in corpus_files(root, config):
        doc, new_chunks = chunk_doc(path, root, cfg, len(chunks) + 1)
        docs.append(doc)
        chunks.extend(new_chunks)
    return {
        "schema_version": 1,
        "index_version": f"{prefix}:{model}:{cfg_hash}",
        "embedding_model_id": model,
        "chunking": {**cfg, "chunk_config_hash": cfg_hash},
        "index_metadata": {
            "reembed_policy": "clean_reembed_on_model_or_chunk_config_change",
            "version_tuple": {"embedding_model_id": model, "chunk_config_hash": cfg_hash},
            "generated_by": "scripts/rag-workstream.sh ingest-index",
        },
        "docs": docs,
        "chunks": chunks,
    }


def main():
    parser = argparse.ArgumentParser(prog="rag-workstream.sh ingest-index")
    parser.add_argument("--config", required=True)
    parser.add_argument("--root", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    config = json.load(open(args.config, encoding="utf-8"))
    cfg, error = validate_config(config)
    if error:
        print(error)
        return 1
    index = build_index(config, Path(args.root), cfg)
    write_json(Path(args.out), index)
    print(f"rag ingest-index wrote {len(index['chunks'])} chunks from {len(index['docs'])} docs to {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
