#!/usr/bin/env python3
"""CLI for deterministic local hybrid retrieval in the RAG workstream."""
import argparse
import json
from typing import Iterable

from rag_retrieve_core import evaluate, retrieve
from rag_query_router import route_evaluate


def read_json_file(path: str):
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def emit_json_document(obj):
    print(json.dumps(obj, indent=2, sort_keys=True))


def parse_filter(items: Iterable[str]):
    filters = {}
    for item in items:
        key, value = item.split("=", 1) if "=" in item else (_raise_filter(item), "")
        filters[key] = value
    return filters


def _raise_filter(item: str):
    raise argparse.ArgumentTypeError(f"--filter expects KEY=VALUE: {item}")


def add_knobs(parser):
    parser.add_argument("--dense-top-k", type=int)
    parser.add_argument("--bm25-top-k", type=int)
    parser.add_argument("--rrf-k", type=int)
    parser.add_argument("--fusion-weight", type=float)
    parser.add_argument("--rerank-top-n", type=int)
    parser.add_argument("--final-n", type=int)


def overrides(args):
    return {"dense_top_k": args.dense_top_k, "bm25_top_k": args.bm25_top_k, "rrf_k": args.rrf_k, "fusion_weight": args.fusion_weight, "rerank_top_n": args.rerank_top_n, "final_n": args.final_n}


def build_cli_parser():
    parser = argparse.ArgumentParser(prog="rag_retrieve.py")
    sub = parser.add_subparsers(dest="cmd", required=True)
    retrieve_parser = sub.add_parser("retrieve")
    add_common_args(retrieve_parser)
    retrieve_parser.add_argument("--query", required=True)
    retrieve_parser.add_argument("--filter", action="append", default=[])
    eval_parser = sub.add_parser("retrieve-eval")
    add_common_args(eval_parser)
    eval_parser.add_argument("--golden", required=True)
    router_eval_parser = sub.add_parser("query-router-eval")
    add_common_args(router_eval_parser)
    router_eval_parser.add_argument("--golden", required=True)
    return parser


def add_common_args(parser):
    parser.add_argument("--index", required=True)
    parser.add_argument("--config", required=True)
    parser.add_argument("--mode", choices=["hybrid", "dense", "bm25"], default="hybrid")
    add_knobs(parser)


def main(argv=None):
    args = build_cli_parser().parse_args(argv)
    index, config = read_json_file(args.index), read_json_file(args.config)
    if args.cmd == "retrieve":
        emit_json_document(retrieve(index, config, args.query, parse_filter(args.filter), mode=args.mode, overrides=overrides(args)))
    elif args.cmd == "retrieve-eval":
        emit_json_document(evaluate(index, config, read_json_file(args.golden), args.mode, overrides(args)))
    else:
        emit_json_document(route_evaluate(index, config, read_json_file(args.golden), args.mode, overrides(args)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
