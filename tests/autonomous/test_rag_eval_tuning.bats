#!/usr/bin/env bats
# tests/rag-eval-tuning.bats — golden-set eval + unattended tuning gate for issue #1551.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/rag-workstream.sh"
    WORK="$(mktemp -d -t rag-eval-tuning.XXXXXX)"
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

write_eval_fixture() {
    cat > "$WORK/config.json" <<'JSON'
{"corpus":{"include":["docs/**/*.md"]},"chunking":{"strategy":"structure_aware","late_chunking":false,"contextual_prefix":false,"chunk_size":900,"overlap":0},"embedding":{"model_id":"local-test-embed","index_version_prefix":"rag-index-v1"},"retrieval":{"dense_top_k":4,"bm25_top_k":4,"rrf_k":60,"fusion_weight":0.50,"rerank_top_n":4,"final_n":2},"query_transform":{"rewrite":false,"hyde":false,"multi_query":false},"freshness":{"serve_latest":true,"invalidate_by":["doc_id","content_hash"]},"eval":{"target_metric":"ndcg","ragas_faithfulness_floor":0.90,"heldout_fold":1,"heldout_modulus":4}}
JSON
    cat > "$WORK/index.json" <<'JSON'
{"schema_version":1,"index_version":"rag-index-v1:test","chunks":[
{"chunk_id":"install","doc_id":"install","source_path":"docs/install.md","heading":"Install","text":"Install autospec with the bootstrap script before running agents.","metadata":{"section":"setup"}},
{"chunk_id":"configure","doc_id":"configure","source_path":"docs/configure.md","heading":"Configure","text":"Set AUTOSPEC_HOME so run state and process heartbeats are stored consistently.","metadata":{"section":"setup"}},
{"chunk_id":"rrf","doc_id":"rrf","source_path":"docs/rag.md","heading":"RRF","text":"Hybrid retrieval combines BM25 and dense rankings with reciprocal rank fusion.","metadata":{"section":"search"}}]}
JSON
    cat > "$WORK/golden.json" <<'JSON'
{"schema_version":1,"version":"test-golden-v1","heldout":{"fold":1,"modulus":4},"queries":[
{"id":"q1","question":"How do I install autospec?","ideal_answer":"Install autospec with the bootstrap script.","relevant_chunk_ids":["install"],"filters":{"section":"setup"}},
{"id":"q2","question":"Where is AUTOSPEC_HOME used?","ideal_answer":"AUTOSPEC_HOME stores run state and process heartbeats.","relevant_chunk_ids":["configure"],"filters":{"section":"setup"}},
{"id":"q3","question":"How does hybrid retrieval combine ranks?","ideal_answer":"Hybrid retrieval combines BM25 and dense rankings with reciprocal rank fusion.","relevant_chunk_ids":["rrf"],"filters":{"section":"search"}},
{"id":"q4","question":"Which chunk mentions process heartbeats?","ideal_answer":"Process heartbeats are stored under AUTOSPEC_HOME.","relevant_chunk_ids":["configure"],"filters":{"section":"setup"}}]}
JSON
}

@test "eval-golden emits deterministic retrieval, held-out, and RAGAS contract scores" {
    write_eval_fixture
    run bash "$SCRIPT" eval-golden --index "$WORK/index.json" --config "$WORK/config.json" --golden "$WORK/golden.json" --out "$WORK/eval.json"
    [ "$status" -eq 0 ]
    [[ "$output" == *"rag eval-golden wrote"* ]]
    python3 - "$WORK/eval.json" <<'PY'
import json, sys
out=json.load(open(sys.argv[1]))
assert out["golden_set"]["version"] == "test-golden-v1", out
assert out["golden_set"]["train_queries"] == 3, out
assert out["golden_set"]["heldout_queries"] == 1, out
assert out["retrieval"]["queries"] == 4, out
for key in ("precision_at_k", "recall_at_k", "mrr", "ndcg"):
    assert key in out["retrieval"], out
for key in ("faithfulness", "answer_relevancy", "context_precision", "context_recall"):
    assert key in out["ragas"], out
assert out["ragas"]["faithfulness"] >= 0.90, out
assert len(out["per_query"]) == 4, out
PY
    run bash "$SCRIPT" eval-golden --index "$WORK/index.json" --config "$WORK/config.json" --golden "$WORK/golden.json"
    [ "$status" -eq 0 ]
    first="$output"
    run bash "$SCRIPT" eval-golden --index "$WORK/index.json" --config "$WORK/config.json" --golden "$WORK/golden.json"
    [ "$status" -eq 0 ]
    [ "$output" = "$first" ]
}

@test "auto-tune records audit scores, promotes positive nDCG, and quarantines faithfulness regression" {
    write_eval_fixture
    python3 - "$WORK/config.json" "$WORK/base-config.json" "$WORK/good-config.json" "$WORK/bad-config.json" <<'PY'
import json, sys
base=json.load(open(sys.argv[1])); base["retrieval"]["fusion_weight"]=1.0; base["retrieval"]["final_n"]=1
good=json.loads(json.dumps(base)); good["retrieval"]["fusion_weight"]=0.0; good["retrieval"]["final_n"]=2
bad=json.loads(json.dumps(good)); bad["ragas"]={"deterministic_faithfulness": 0.80}
for path, obj in [(sys.argv[2], base), (sys.argv[3], good), (sys.argv[4], bad)]:
    json.dump(obj, open(path,"w"), indent=2, sort_keys=True)
PY
    cat > "$WORK/tune-index.json" <<'JSON'
{"schema_version":1,"index_version":"rag-index-v1:test","chunks":[
{"chunk_id":"dense-decoy","doc_id":"semantic","source_path":"docs/semantic.md","heading":"Semantic","text":"Semantic vector search handles broad documentation concepts.","metadata":{"section":"search"}},
{"chunk_id":"rrf-target","doc_id":"rrf","source_path":"docs/rrf.md","heading":"RRF","text":"The rrf_k knob configures reciprocal rank fusion for exact BM25 jargon.","metadata":{"section":"search"}},
{"chunk_id":"install-target","doc_id":"install","source_path":"docs/install.md","heading":"Install","text":"Install autospec with the bootstrap script.","metadata":{"section":"setup"}}]}
JSON
    cat > "$WORK/tune-golden.json" <<'JSON'
{"schema_version":1,"version":"tune-v1","queries":[
{"id":"q1","question":"rrf_k","ideal_answer":"The rrf_k knob configures reciprocal rank fusion.","relevant_chunk_ids":{"rrf-target":3},"filters":{"section":"search"}},
{"id":"q2","question":"install autospec","ideal_answer":"Install autospec with the bootstrap script.","relevant_chunk_ids":{"install-target":3},"filters":{"section":"setup"}}]}
JSON
    run bash "$SCRIPT" auto-tune --index "$WORK/tune-index.json" --config "$WORK/base-config.json" --golden "$WORK/tune-golden.json" --candidate-config "$WORK/good-config.json" --audit-log "$WORK/audit.ndjson" --promote-out "$WORK/promoted.json" --quarantine-out "$WORK/quarantine.json"
    [ "$status" -eq 0 ]
    [[ "$output" == *"rag auto-tune promoted"* ]]
    [ "$(wc -l < "$WORK/audit.ndjson" | tr -d ' ')" -eq 2 ]
    python3 - "$WORK/promoted.json" <<'PY'
import json, sys
prom=json.load(open(sys.argv[1]))
assert prom["decision"] == "promoted", prom
assert prom["candidate"]["retrieval"]["ndcg"] > prom["baseline"]["retrieval"]["ndcg"], prom
PY
    rm -f "$WORK/promoted.json" "$WORK/quarantine.json" "$WORK/audit.ndjson"
    run bash "$SCRIPT" auto-tune --index "$WORK/tune-index.json" --config "$WORK/base-config.json" --golden "$WORK/tune-golden.json" --candidate-config "$WORK/bad-config.json" --audit-log "$WORK/audit.ndjson" --promote-out "$WORK/promoted.json" --quarantine-out "$WORK/quarantine.json"
    [ "$status" -eq 1 ]
    [[ "$output" == *"RAGAS_FAITHFULNESS_FLOOR"* ]]
    [[ "$output" == *"rag auto-tune quarantined"* ]]
    [ ! -f "$WORK/promoted.json" ]
    [ -f "$WORK/quarantine.json" ]
}
