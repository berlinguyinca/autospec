import hashlib

from rag_ingest_blocks import split_section
from rag_ingest_common import repo_rel, text_sha256
from rag_ingest_corpus import corpus_files
from rag_ingest_markdown import markdown_sections


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


def chunk_doc(path, root, cfg, first_ordinal):
    rel, text = repo_rel(path, root), path.read_text(encoding="utf-8")
    doc_hash = text_sha256(text)
    pairs = section_pairs(text, cfg)
    chunks = [make_chunk(rel, doc_hash, part, heading, first_ordinal + i, cfg, text)
              for i, (heading, part) in enumerate(pairs)]
    return {"doc_id": rel, "path": rel, "content_hash": doc_hash, "chunk_count": len(chunks)}, chunks


def build_index(config, root, cfg, cfg_hash):
    model = str(config.get("embedding", {}).get("model_id", "local-deterministic"))
    prefix = config.get("embedding", {}).get("index_version_prefix", "rag-index-v1")
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
