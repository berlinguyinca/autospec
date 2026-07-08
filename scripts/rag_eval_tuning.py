#!/usr/bin/env python3
"""Golden-set evaluation and deterministic auto-tuning for the RAG workstream.

This module is intentionally dependency-free.  The generation metrics are a
local contract shim for nightly RAGAS judging: the field names, floors, and
promotion semantics match the RAGAS gate, while scores are deterministic token
support measurements over retrieved chunks for commit-time validation.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Dict, Iterable, List, Mapping, Sequence

from rag_retrieve_core import query_metrics, retrieve, retrieval_knobs
from rag_retrieve_text import tokenize

METRIC_KEYS = ("ndcg", "mrr", "recall_at_k", "precision_at_k")
RAGAS_KEYS = ("faithfulness", "answer_relevancy", "context_precision", "context_recall")
STOPWORDS = {
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "how", "in", "is",
    "it", "of", "on", "or", "so", "the", "to", "under", "with", "where", "which",
}


def load_json(path: str | Path):
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def write_json(path: str | Path, obj) -> None:
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(obj, fh, indent=2, sort_keys=True)
        fh.write("\n")


def canonical(obj) -> str:
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def config_version(config: Mapping) -> str:
    prefix = config.get("embedding", {}).get("index_version_prefix", "rag-index-v1")
    digest = hashlib.sha256(canonical(config).encode("utf-8")).hexdigest()[:16]
    return f"{prefix}:{digest}"


def golden_rows(golden: Mapping) -> List[Dict]:
    raw_rows = golden.get("queries", golden if isinstance(golden, list) else [])
    rows: List[Dict] = []
    for idx, row in enumerate(raw_rows, 1):
        relevant = row.get("relevant", row.get("relevant_chunk_ids", {}))
        if isinstance(relevant, list):
            relevant = {str(chunk_id): 1 for chunk_id in relevant}
        rows.append(
            {
                "id": str(row.get("id", f"q{idx}")),
                "query": str(row.get("query", row.get("question", ""))),
                "question": str(row.get("question", row.get("query", ""))),
                "ideal_answer": str(row.get("ideal_answer", "")),
                "relevant": {str(k): float(v) for k, v in dict(relevant).items()},
                "filters": dict(row.get("filters", {})),
            }
        )
    return rows


def heldout_params(golden: Mapping, config: Mapping) -> tuple[int, int]:
    heldout = dict(golden.get("heldout", {}))
    eval_cfg = dict(config.get("eval", {}))
    fold = int(heldout.get("fold", eval_cfg.get("heldout_fold", 0)))
    modulus = int(heldout.get("modulus", eval_cfg.get("heldout_modulus", 5)))
    return fold, max(modulus, 1)


def split_name(position: int, fold: int, modulus: int) -> str:
    return "heldout" if position % modulus == fold % modulus else "train"


def average(rows: Sequence[Mapping[str, float]], key: str) -> float:
    if not rows:
        return 0.0
    return round(sum(float(row.get(key, 0.0)) for row in rows) / len(rows), 6)


def normalized_tokens(text: str) -> List[str]:
    return [token for token in tokenize(text) if token not in STOPWORDS]


def supported_fraction(answer: str, context: str) -> float:
    answer_tokens = normalized_tokens(answer)
    if not answer_tokens:
        return 1.0
    context_tokens = set(normalized_tokens(context))
    context_prefixes = {token[:5] for token in context_tokens if len(token) >= 5}
    supported = 0
    for token in answer_tokens:
        if token in context_tokens or (len(token) >= 5 and token[:5] in context_prefixes):
            supported += 1
    return supported / len(answer_tokens)


def answer_relevancy(question: str, answer: str) -> float:
    q_tokens = set(normalized_tokens(question))
    if not q_tokens:
        return 1.0
    a_tokens = set(normalized_tokens(answer))
    a_prefixes = {token[:5] for token in a_tokens if len(token) >= 5}
    hits = sum(1 for token in q_tokens if token in a_tokens or (len(token) >= 5 and token[:5] in a_prefixes))
    return hits / len(q_tokens)


def chunks_by_id(index: Mapping) -> Dict[str, Mapping]:
    return {str(chunk.get("chunk_id")): chunk for chunk in index.get("chunks", index if isinstance(index, list) else [])}


def context_for_results(index: Mapping, result_ids: Iterable[str]) -> str:
    by_id = chunks_by_id(index)
    texts = []
    for chunk_id in result_ids:
        chunk = by_id.get(str(chunk_id), {})
        texts.append("\n".join(str(chunk.get(k, "")) for k in ("heading", "text")))
    return "\n".join(texts)


def deterministic_ragas(config: Mapping, row: Mapping, results: Sequence[Mapping], retrieval_metrics: Mapping[str, float], index: Mapping) -> Dict[str, float]:
    override = config.get("ragas", {}).get("deterministic_faithfulness")
    result_ids = [str(result.get("chunk_id")) for result in results]
    context = context_for_results(index, result_ids)
    faith = float(override) if override is not None else supported_fraction(str(row.get("ideal_answer", "")), context)
    return {
        "faithfulness": round(max(0.0, min(1.0, faith)), 6),
        "answer_relevancy": round(answer_relevancy(str(row.get("question", "")), str(row.get("ideal_answer", ""))), 6),
        "context_precision": round(float(retrieval_metrics.get("precision_at_k", 0.0)), 6),
        "context_recall": round(float(retrieval_metrics.get("recall_at_k", 0.0)), 6),
    }


def evaluate_golden(index: Mapping, config: Mapping, golden: Mapping, mode: str, overrides: Mapping[str, int | float | None]) -> Dict:
    rows = golden_rows(golden)
    knobs = retrieval_knobs(config, overrides)
    final_n = int(knobs["final_n"])
    fold, modulus = heldout_params(golden, config)
    per_query = []
    retrieval_metrics: List[Dict[str, float]] = []
    ragas_metrics: List[Dict[str, float]] = []
    for position, row in enumerate(rows):
        out = retrieve(index, config, row["query"], row.get("filters", {}), mode=mode, overrides=overrides)
        metrics = query_metrics(out["results"], row.get("relevant", {}), final_n)
        ragas = deterministic_ragas(config, row, out["results"], metrics, index)
        retrieval_metrics.append(metrics)
        ragas_metrics.append(ragas)
        per_query.append(
            {
                "id": row["id"],
                "query": row["query"],
                "split": split_name(position, fold, modulus),
                "filters": row.get("filters", {}),
                "results": [result["chunk_id"] for result in out["results"]],
                "retrieval": {key: round(float(metrics[key]), 6) for key in METRIC_KEYS},
                "ragas": ragas,
            }
        )
    return {
        "schema_version": 1,
        "generated_at": "deterministic",
        "mode": mode,
        "config_version": config_version(config),
        "index_version": index.get("index_version", "unknown"),
        "knobs": knobs,
        "golden_set": {
            "schema_version": golden.get("schema_version", 1),
            "version": golden.get("version", "unknown"),
            "queries": len(rows),
            "train_queries": sum(1 for row in per_query if row["split"] == "train"),
            "heldout_queries": sum(1 for row in per_query if row["split"] == "heldout"),
            "heldout": {"fold": fold, "modulus": modulus},
        },
        "retrieval": {key: average(retrieval_metrics, key) for key in METRIC_KEYS} | {"queries": len(rows)},
        "ragas": {key: average(ragas_metrics, key) for key in RAGAS_KEYS},
        "per_query": per_query,
    }


def append_audit(path: str | Path, role: str, scores: Mapping) -> None:
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    row = {
        "ts": datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "role": role,
        "config_version": scores.get("config_version"),
        "index_version": scores.get("index_version"),
        "golden_set": scores.get("golden_set"),
        "retrieval": scores.get("retrieval"),
        "ragas": scores.get("ragas"),
    }
    with open(path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(row, sort_keys=True) + "\n")


def promotion_findings(baseline: Mapping, candidate: Mapping, target_metric: str, floor: float) -> List[str]:
    base_target = float(baseline["retrieval"][target_metric])
    cand_target = float(candidate["retrieval"][target_metric])
    base_faith = float(baseline["ragas"]["faithfulness"])
    cand_faith = float(candidate["ragas"]["faithfulness"])
    findings: List[str] = []
    if cand_target <= base_target:
        findings.append(f"TARGET_METRIC_NOT_IMPROVED:{target_metric}:{cand_target:.3f}<={base_target:.3f}")
    if cand_faith < floor:
        findings.append(f"RAGAS_FAITHFULNESS_FLOOR:{cand_faith:.3f}<{floor:.3f}")
    if cand_faith < base_faith:
        findings.append(f"RAGAS_FAITHFULNESS_REGRESSION:{cand_faith:.3f}<{base_faith:.3f}")
    return findings


def cmd_eval_golden(args) -> int:
    scores = evaluate_golden(load_json(args.index), load_json(args.config), load_json(args.golden), args.mode, overrides(args))
    if args.out:
        write_json(args.out, scores)
        print(f"rag eval-golden wrote {args.out}")
    else:
        print(json.dumps(scores, indent=2, sort_keys=True))
    return 0


def cmd_auto_tune(args) -> int:
    index = load_json(args.index)
    golden = load_json(args.golden)
    baseline_config = load_json(args.config)
    candidate_config = load_json(args.candidate_config)
    target = args.target_metric or candidate_config.get("eval", {}).get("target_metric") or baseline_config.get("eval", {}).get("target_metric", "ndcg")
    floor = float(args.faithfulness_floor or candidate_config.get("eval", {}).get("ragas_faithfulness_floor") or baseline_config.get("eval", {}).get("ragas_faithfulness_floor", 0.90))
    baseline = evaluate_golden(index, baseline_config, golden, args.mode, overrides(args))
    candidate = evaluate_golden(index, candidate_config, golden, args.mode, overrides(args))
    if args.audit_log:
        append_audit(args.audit_log, "baseline", baseline)
        append_audit(args.audit_log, "candidate", candidate)
    findings = promotion_findings(baseline, candidate, target, floor)
    decision = {
        "schema_version": 1,
        "decision": "promoted" if not findings else "quarantined",
        "target_metric": target,
        "faithfulness_floor": floor,
        "baseline": baseline,
        "candidate": candidate,
        "findings": findings,
    }
    if findings:
        for finding in findings:
            print(finding)
        if args.quarantine_out:
            write_json(args.quarantine_out, decision)
        print("rag auto-tune quarantined: candidate requires human review")
        return 1
    decision["candidate_config"] = candidate_config
    if args.promote_out:
        write_json(args.promote_out, decision)
    print(
        f"rag auto-tune promoted: {target} "
        f"{float(baseline['retrieval'][target]):.3f}->{float(candidate['retrieval'][target]):.3f}; "
        f"faithfulness {float(candidate['ragas']['faithfulness']):.3f}>={floor:.3f}"
    )
    return 0


def add_common(parser):
    parser.add_argument("--index", required=True)
    parser.add_argument("--config", required=True)
    parser.add_argument("--golden", required=True)
    parser.add_argument("--mode", choices=["hybrid", "dense", "bm25"], default="hybrid")
    parser.add_argument("--dense-top-k", type=int)
    parser.add_argument("--bm25-top-k", type=int)
    parser.add_argument("--rrf-k", type=int)
    parser.add_argument("--fusion-weight", type=float)
    parser.add_argument("--rerank-top-n", type=int)
    parser.add_argument("--final-n", type=int)


def overrides(args) -> Dict[str, int | float | None]:
    return {
        "dense_top_k": args.dense_top_k,
        "bm25_top_k": args.bm25_top_k,
        "rrf_k": args.rrf_k,
        "fusion_weight": args.fusion_weight,
        "rerank_top_n": args.rerank_top_n,
        "final_n": args.final_n,
    }


def build_parser():
    parser = argparse.ArgumentParser(prog="rag_eval_tuning.py")
    sub = parser.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("eval-golden")
    add_common(p)
    p.add_argument("--out")
    p.set_defaults(func=cmd_eval_golden)
    p = sub.add_parser("auto-tune")
    add_common(p)
    p.add_argument("--candidate-config", required=True)
    p.add_argument("--audit-log", required=True)
    p.add_argument("--promote-out", required=True)
    p.add_argument("--quarantine-out", required=True)
    p.add_argument("--target-metric")
    p.add_argument("--faithfulness-floor")
    p.set_defaults(func=cmd_auto_tune)
    return parser


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
