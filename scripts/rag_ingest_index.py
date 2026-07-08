import hashlib

from rag_ingest_blocks import split_section
from rag_ingest_common import repo_rel, text_sha256
from rag_ingest_corpus import corpus_files
from rag_ingest_markdown import markdown_sections


def contextual_prefix(rel, heading, ordinal):
    location = f"Document: {rel}" + (f" > {heading}" if heading and heading != "Document" else "")
    return f"{location}. Chunk {ordinal} context: use this passage as part of the autospec documentation corpus."


def make_chunk(rel, doc_hash, text, heading, ordinal, cfg, full_text, doc_version=1, is_latest=True):
    prefix = contextual_prefix(rel, heading, ordinal) if cfg["contextual_prefix"] else ""
    basis = full_text if cfg["late_chunking"] else ((prefix + "\n") if prefix else "") + text
    record = {
        "chunk_id": hashlib.sha256(f"{rel}\0{doc_hash}\0{doc_version}\0{ordinal}\0{text}".encode("utf-8")).hexdigest()[:16],
        "doc_id": rel,
        "source_path": rel,
        "doc_content_hash": doc_hash,
        "doc_version": int(doc_version),
        "is_latest": bool(is_latest),
        "ordinal": ordinal,
        "heading": heading,
        "text": text,
        "text_hash": text_sha256(text),
        "embedding_input_hash": text_sha256(basis),
        "strategy": cfg["strategy"],
        "late_chunking": cfg["late_chunking"],
    }
    record.update({"contextual_prefix": prefix} if prefix else {})
    return record


def section_pairs(text, cfg):
    pairs = []
    for heading, section_text in markdown_sections(text):
        for part in split_section(section_text, cfg["chunk_size"], cfg["overlap"]):
            nonblank = [line.strip() for line in part.splitlines() if line.strip()]
            pairs.extend([] if (heading and len(nonblank) <= 1) else [(heading, part)])
    return pairs


def chunk_doc(path, root, cfg, first_ordinal, doc_version=1, is_latest=True):
    rel, text = repo_rel(path, root), path.read_text(encoding="utf-8")
    doc_hash = text_sha256(text)
    pairs = section_pairs(text, cfg)
    chunks = [make_chunk(rel, doc_hash, part, heading, first_ordinal + i, cfg, text, doc_version, is_latest)
              for i, (heading, part) in enumerate(pairs)]
    return {"doc_id": rel, "path": rel, "content_hash": doc_hash, "latest_content_hash": doc_hash, "doc_version": int(doc_version), "latest_version": int(doc_version), "is_latest": bool(is_latest), "chunk_count": len(chunks)}, chunks


def _docs_by_id(docs):
    grouped = {}
    for doc in docs:
        doc_id = str(doc.get("doc_id"))
        version = int(doc.get("latest_version", doc.get("doc_version", 1)))
        current = grouped.get(doc_id)
        if current is None or version > int(current.get("latest_version", current.get("doc_version", 1))):
            grouped[doc_id] = {
                "path": doc.get("path", doc_id),
                "latest_version": version,
                "latest_content_hash": doc.get("latest_content_hash", doc.get("content_hash", "")),
                "versions": sorted(set(((current or {}).get("versions", [])) + [version])),
            }
        else:
            grouped[doc_id]["versions"] = sorted(set([*grouped[doc_id].get("versions", []), version]))
    return grouped


def _with_docs_by_id(index):
    index["docs_by_id"] = _docs_by_id(index.get("docs", []))
    return index


def _base_index(config, cfg, cfg_hash):
    model = str(config.get("embedding", {}).get("model_id", "local-deterministic"))
    prefix = config.get("embedding", {}).get("index_version_prefix", "rag-index-v1")
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
        "docs": [],
        "chunks": [],
    }


def build_index(config, root, cfg, cfg_hash):
    index = _base_index(config, cfg, cfg_hash)
    for path in corpus_files(root, config):
        doc, new_chunks = chunk_doc(path, root, cfg, len(index["chunks"]) + 1)
        index["docs"].append(doc)
        index["chunks"].extend(new_chunks)
    return _with_docs_by_id(index)


def build_incremental_index(config, root, cfg, cfg_hash, previous, changed_docs=None):
    current_paths = {repo_rel(path, root): path for path in corpus_files(root, config)}
    requested = set(changed_docs or [])
    previous_latest = previous.get("docs_by_id") or _docs_by_id(previous.get("docs", []))
    changed = []
    for rel, path in current_paths.items():
        current_hash = text_sha256(path.read_text(encoding="utf-8"))
        old_hash = str(previous_latest.get(rel, {}).get("latest_content_hash", ""))
        if rel in requested or old_hash != current_hash:
            changed.append(rel)
    index = _base_index(config, cfg, cfg_hash)
    changed_set = set(changed)
    invalidated_hashes = {}
    reused_chunks = []
    for chunk in previous.get("chunks", []):
        rel = str(chunk.get("doc_id"))
        if rel in changed_set:
            old = dict(chunk)
            old["is_latest"] = False
            reused_chunks.append(old)
            invalidated_hashes.setdefault(rel, set()).add(str(old.get("doc_content_hash", "")))
        elif rel in current_paths:
            reused_chunks.append(chunk)
    index["chunks"].extend(reused_chunks)

    docs = []
    # Preserve prior doc versions, but mark changed prior latest docs as non-latest.
    for doc in previous.get("docs", []):
        rel = str(doc.get("doc_id"))
        if rel not in current_paths:
            continue
        preserved = dict(doc)
        if rel in changed_set:
            preserved["is_latest"] = False
        docs.append(preserved)
    for rel in changed:
        latest_version = int(previous_latest.get(rel, {}).get("latest_version", 0)) + 1
        doc, new_chunks = chunk_doc(current_paths[rel], root, cfg, len(index["chunks"]) + 1, latest_version, True)
        docs.append(doc)
        index["chunks"].extend(new_chunks)
    if not previous.get("docs"):
        for rel, path in current_paths.items():
            if rel not in changed_set:
                doc, new_chunks = chunk_doc(path, root, cfg, len(index["chunks"]) + 1)
                docs.append(doc)
                index["chunks"].extend(new_chunks)
    index["docs"] = docs
    _with_docs_by_id(index)
    index["incremental_reindex"] = {
        "changed_docs": changed,
        "invalidated_doc_hashes": {k: sorted(v) for k, v in sorted(invalidated_hashes.items())},
        "reembedded_chunk_count": sum(1 for c in index["chunks"] if str(c.get("doc_id")) in changed_set and c.get("is_latest")),
        "reused_chunk_count": sum(1 for c in reused_chunks if str(c.get("doc_id")) not in changed_set),
        "invalidated_chunk_count": sum(1 for c in reused_chunks if str(c.get("doc_id")) in changed_set),
    }
    return index
