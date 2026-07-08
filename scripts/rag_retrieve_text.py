"""Text and metadata helpers for deterministic RAG retrieval."""
import hashlib
import re
from collections import Counter
from typing import Dict, List, Mapping

TOKEN_RE = re.compile(r"[a-z0-9_]+")


def tokenize(text: str) -> List[str]:
    return TOKEN_RE.findall(str(text).lower())


def dense_tokens(text: str) -> List[str]:
    buckets: List[str] = []
    for token in tokenize(text):
        if "_" not in token and len(token) >= 5:
            buckets.append(f"{token[:4]}:{hashlib.sha256(token.encode('utf-8')).hexdigest()[:2]}")
    return buckets


def chunk_text(chunk: Mapping) -> str:
    return "\n".join(str(chunk.get(k, "")) for k in ("heading", "text", "source_path", "doc_id"))


def chunk_metadata(chunk: Mapping) -> Dict[str, str]:
    metadata = {str(k): str(v) for k, v in dict(chunk.get("metadata", {})).items()}
    metadata.update({key: str(chunk[key]) for key in ("index_version", "doc_version", "section", "product_area") if key in chunk and key not in metadata})
    return metadata


def candidate_chunks(index: Mapping, filters: Mapping[str, str]):
    chunks = index.get("chunks", index if isinstance(index, list) else [])
    latest_default = bool(index.get("docs_by_id")) and "doc_version" not in filters
    rows = []
    for chunk in chunks:
        if latest_default and chunk.get("is_latest") is False:
            continue
        metadata = chunk_metadata(chunk)
        if all(str(metadata.get(key, "")) == str(value) for key, value in filters.items()):
            rows.append(chunk)
    return rows


def count_overlap(query_counts: Counter, doc_counts: Counter) -> float:
    return float(sum(min(count, doc_counts.get(tok, 0)) for tok, count in query_counts.items()))
