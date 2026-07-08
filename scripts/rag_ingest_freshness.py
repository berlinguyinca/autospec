"""Incremental freshness/versioning helpers for deterministic RAG indexes."""

from rag_ingest_common import repo_rel, text_sha256
from rag_ingest_corpus import corpus_files
from rag_ingest_index import _base_index, _docs_by_id, _with_docs_by_id, chunk_doc


def changed_doc_paths(config, root, previous, changed_docs=None):
    current_paths = {repo_rel(path, root): path for path in corpus_files(root, config)}
    requested = set(changed_docs or [])
    previous_latest = previous.get("docs_by_id") or _docs_by_id(previous.get("docs", []))
    changed = [
        rel for rel, path in current_paths.items()
        if rel in requested or previous_latest.get(rel, {}).get("latest_content_hash", "") != text_sha256(path.read_text(encoding="utf-8"))
    ]
    return current_paths, previous_latest, changed


def carry_forward_chunks(previous, current_paths, changed_set):
    invalidated_hashes = {}
    kept = []
    for chunk in previous.get("chunks", []):
        rel = str(chunk.get("doc_id"))
        if rel not in current_paths:
            continue
        row = dict(chunk, is_latest=False) if rel in changed_set else chunk
        kept.append(row)
        invalidated_hashes.setdefault(rel, set()).add(str(row.get("doc_content_hash", ""))) if rel in changed_set else None
    return kept, invalidated_hashes


def carry_forward_docs(previous, current_paths, changed_set):
    return [
        dict(doc, is_latest=False) if str(doc.get("doc_id")) in changed_set else dict(doc)
        for doc in previous.get("docs", [])
        if str(doc.get("doc_id")) in current_paths
    ]


def add_changed_versions(index, docs, current_paths, previous_latest, changed, root, cfg):
    for rel in changed:
        latest_version = int(previous_latest.get(rel, {}).get("latest_version", 0)) + 1
        doc, chunks = chunk_doc(current_paths[rel], root, cfg, len(index["chunks"]) + 1, latest_version, True)
        docs.append(doc)
        index["chunks"].extend(chunks)


def add_missing_full_rebuild_docs(index, docs, current_paths, changed_set, root, cfg):
    for rel, path in current_paths.items():
        if rel not in changed_set:
            doc, chunks = chunk_doc(path, root, cfg, len(index["chunks"]) + 1)
            docs.append(doc)
            index["chunks"].extend(chunks)


def build_incremental_index(config, root, cfg, cfg_hash, previous, changed_docs=None):
    current_paths, previous_latest, changed = changed_doc_paths(config, root, previous, changed_docs)
    index = _base_index(config, cfg, cfg_hash)
    changed_set = set(changed)
    reused_chunks, invalidated_hashes = carry_forward_chunks(previous, current_paths, changed_set)
    index["chunks"].extend(reused_chunks)
    docs = carry_forward_docs(previous, current_paths, changed_set)
    add_changed_versions(index, docs, current_paths, previous_latest, changed, root, cfg)
    add_missing_full_rebuild_docs(index, docs, current_paths, changed_set, root, cfg) if not previous.get("docs") else None
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
