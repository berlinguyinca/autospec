import hashlib

from rag_ingest_common import index_canonical

REQUIRED_CONFIG_SECTIONS = ["corpus", "chunking", "embedding", "retrieval", "query_transform", "freshness", "eval"]


def chunk_settings(config):
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


def validate_chunk_settings(config):
    missing = [section for section in REQUIRED_CONFIG_SECTIONS if section not in config]
    if missing:
        return None, "CONFIG_SECTION_MISSING:" + ",".join(missing)
    cfg = chunk_settings(config)
    if cfg["strategy"] != "structure_aware":
        return None, f"UNSUPPORTED_CHUNK_STRATEGY:{cfg['strategy']}"
    if cfg["chunk_size"] <= 0:
        return None, "CHUNK_SIZE_INVALID:must_be_positive"
    if cfg["overlap"] < 0 or cfg["overlap"] >= cfg["chunk_size"]:
        return None, "CHUNK_OVERLAP_INVALID:must_be_nonnegative_and_less_than_chunk_size"
    return cfg, None


def chunk_settings_hash(cfg):
    return hashlib.sha256(index_canonical(cfg).encode("utf-8")).hexdigest()[:16]
