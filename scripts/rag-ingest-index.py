#!/usr/bin/env python3
"""Deterministic local RAG ingestion/index builder for issue #1548."""
import argparse
import json
from pathlib import Path

from rag_ingest_common import write_index_json
from rag_ingest_config import chunk_settings_hash, validate_chunk_settings
from rag_ingest_index import build_index


def parse_args():
    parser = argparse.ArgumentParser(prog="rag-workstream.sh ingest-index")
    parser.add_argument("--config", required=True)
    parser.add_argument("--root", required=True)
    parser.add_argument("--out", required=True)
    return parser.parse_args()


def main():
    args = parse_args()
    config = json.load(open(args.config, encoding="utf-8"))
    cfg, error = validate_chunk_settings(config)
    if error:
        print(error)
        return 1
    index = build_index(config, Path(args.root), cfg, chunk_settings_hash(cfg))
    write_index_json(Path(args.out), index)
    print(f"rag ingest-index wrote {len(index['chunks'])} chunks from {len(index['docs'])} docs to {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
