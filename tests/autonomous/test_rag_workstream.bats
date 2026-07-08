#!/usr/bin/env bats
# tests/rag-workstream.bats — foundation contract tests for issue #1541 RAG documentation database.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/rag-workstream.sh"
    WORK="$(mktemp -d -t rag-workstream.XXXXXX)"
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

@test "config: deterministic version hash changes only when RAG knobs change" {
    cat > "$WORK/config.json" <<'JSON'
{
  "corpus": {"include": ["llms.txt", "llms-full.txt", "docs/**/*.md"]},
  "chunking": {"strategy": "structure_aware", "late_chunking": true, "contextual_prefix": true, "chunk_size": 900, "overlap": 120},
  "embedding": {"model_id": "jina-embeddings-v2-base-en"},
  "retrieval": {"dense_top_k": 50, "bm25_top_k": 50, "rrf_k": 60, "fusion_weight": 0.50, "rerank_top_n": 50, "final_n": 8},
  "query_transform": {"rewrite": true, "hyde": "routed", "multi_query": "routed"},
  "freshness": {"serve_latest": true, "invalidate_by": ["doc_id", "content_hash"]},
  "eval": {"target_metric": "ndcg", "ragas_faithfulness_floor": 0.90}
}
JSON
    run bash "$SCRIPT" config-version --config "$WORK/config.json"
    [ "$status" -eq 0 ]
    [[ "$output" == rag-index-v1:* ]]
    first="$output"

    python3 - "$WORK/config.json" <<'PY'
import json, sys
p=sys.argv[1]
d=json.load(open(p))
d["retrieval"]["final_n"]=5
json.dump(d, open(p,"w"), sort_keys=True)
PY
    run bash "$SCRIPT" config-version --config "$WORK/config.json"
    [ "$status" -eq 0 ]
    [[ "$output" == rag-index-v1:* ]]
    [ "$output" != "$first" ]
}

@test "eval gate: promotes only score-positive retrieval changes with RAGAS faithfulness floor" {
    cat > "$WORK/baseline.json" <<'JSON'
{"config_version":"rag-index-v1:base","retrieval":{"ndcg":0.70,"mrr":0.60,"recall_at_k":0.72},"ragas":{"faithfulness":0.93,"answer_relevancy":0.81,"context_precision":0.78}}
JSON
    cat > "$WORK/candidate_bad.json" <<'JSON'
{"config_version":"rag-index-v1:bad","retrieval":{"ndcg":0.74,"mrr":0.62,"recall_at_k":0.74},"ragas":{"faithfulness":0.89,"answer_relevancy":0.83,"context_precision":0.80}}
JSON
    run bash "$SCRIPT" gate --baseline "$WORK/baseline.json" --candidate "$WORK/candidate_bad.json" --target-metric ndcg --faithfulness-floor 0.90 --promote-out "$WORK/promote.json"
    [ "$status" -eq 1 ]
    [[ "$output" == *"RAGAS_FAITHFULNESS_FLOOR:0.890<0.900"* ]]
    [ ! -f "$WORK/promote.json" ]

    cat > "$WORK/candidate_good.json" <<'JSON'
{"config_version":"rag-index-v1:good","retrieval":{"ndcg":0.75,"mrr":0.63,"recall_at_k":0.75},"ragas":{"faithfulness":0.94,"answer_relevancy":0.84,"context_precision":0.81}}
JSON
    run bash "$SCRIPT" gate --baseline "$WORK/baseline.json" --candidate "$WORK/candidate_good.json" --target-metric ndcg --faithfulness-floor 0.90 --promote-out "$WORK/promote.json"
    [ "$status" -eq 0 ]
    [[ "$output" == *"rag eval gate passed"* ]]
    grep -q 'rag-index-v1:good' "$WORK/promote.json"
}

@test "guards: stale index, chunk-boundary loss, and hallucinated citations are rejected" {
    mkdir -p "$WORK/docs"
    printf 'current docs\n' > "$WORK/docs/a.md"
    cat > "$WORK/manifest.json" <<'JSON'
{"index_version":"rag-index-v1:test","docs":[{"doc_id":"a","path":"docs/a.md","content_hash":"WRONG"}]}
JSON
    run bash "$SCRIPT" freshness-check --manifest "$WORK/manifest.json" --root "$WORK"
    [ "$status" -eq 1 ]
    [[ "$output" == *"STALE_INDEX:a"* ]]

    cat > "$WORK/chunks.json" <<'JSON'
{"chunks":[{"chunk_id":"c1","doc_id":"a","heading":"Install","text":"# Install"}]}
JSON
    run bash "$SCRIPT" chunk-boundary-check --chunks "$WORK/chunks.json"
    [ "$status" -eq 1 ]
    [[ "$output" == *"CHUNK_BOUNDARY_LOSS:c1"* ]]

    cat > "$WORK/chunks-good.json" <<'JSON'
{"chunks":[{"chunk_id":"c1","doc_id":"a","heading":"Install","text":"# Install\nRun `autospec install` to install the tool."}]}
JSON
    cat > "$WORK/claims.json" <<'JSON'
{"claims":[{"claim":"Autospec installs with the install command.","chunk_id":"c1","supporting_span":"autospec install"},{"claim":"Autospec deploys Kubernetes.","chunk_id":"c1","supporting_span":"kubectl apply"}]}
JSON
    run bash "$SCRIPT" citation-check --claims "$WORK/claims.json" --chunks "$WORK/chunks-good.json"
    [ "$status" -eq 1 ]
    [[ "$output" == *"UNSUPPORTED_CITATION:c1:kubectl apply"* ]]
}

@test "design doc: cites source canon and hands concrete levers to child issues" {
    run bash "$SCRIPT" validate-design-doc --doc "$REPO_ROOT/docs/runbooks/rag-documentation-database.md"
    [ "$status" -eq 0 ]
    [[ "$output" == *"rag design doc validated"* ]]
}

@test "validate.sh auto-globs the RAG workstream bats suite and required artifacts exist" {
    grep -q 'tests/autonomous/\*.bats' "$REPO_ROOT/scripts/validate.sh"
    grep -q 'check_autonomous_phase2_suite' "$REPO_ROOT/scripts/validate.sh"
    [ -f "$REPO_ROOT/.autospec/rag-workstream/config.json" ]
    [ -x "$REPO_ROOT/scripts/rag-workstream.sh" ]
    [ -f "$REPO_ROOT/docs/runbooks/rag-documentation-database.md" ]
}

@test "ingest-index: deterministic structure-aware chunks and versioned local index" {
    mkdir -p "$WORK/docs"
    cat > "$WORK/docs/fixture.md" <<'MD'
# Install
Run the setup command.

```bash
autospec install
```

## Configure
Set `AUTOSPEC_HOME` before running.

Details stay with configure.
MD
    cat > "$WORK/config.json" <<'JSON'
{
  "corpus": {"include": ["docs/**/*.md"]},
  "chunking": {"strategy": "structure_aware", "late_chunking": false, "contextual_prefix": true, "chunk_size": 70, "overlap": 0},
  "embedding": {"model_id": "local-test-embed", "index_version_prefix": "rag-index-v1"},
  "retrieval": {"dense_top_k": 50, "bm25_top_k": 50, "rrf_k": 60, "fusion_weight": 0.50, "rerank_top_n": 50, "final_n": 8},
  "query_transform": {"rewrite": false, "hyde": false, "multi_query": false},
  "freshness": {"serve_latest": true, "invalidate_by": ["doc_id", "content_hash"]},
  "eval": {"target_metric": "ndcg", "ragas_faithfulness_floor": 0.90}
}
JSON
    run bash "$SCRIPT" ingest-index --config "$WORK/config.json" --root "$WORK" --out "$WORK/index.json"
    [ "$status" -eq 0 ]
    [[ "$output" == *"rag ingest-index wrote"* ]]

    python3 - "$WORK/index.json" <<'PY'
import json, sys
idx=json.load(open(sys.argv[1]))
assert idx["embedding_model_id"] == "local-test-embed"
assert idx["chunking"]["chunk_config_hash"]
assert idx["index_version"].startswith("rag-index-v1:local-test-embed:")
assert idx["index_metadata"]["reembed_policy"] == "clean_reembed_on_model_or_chunk_config_change"
chunks=idx["chunks"]
assert len(chunks) >= 2, chunks
assert chunks[0]["heading"] == "Install"
assert "Run the setup command." in chunks[0]["text"], chunks[0]["text"]
assert chunks[0]["text"].count("```") == 2, chunks[0]["text"]
assert chunks[0]["contextual_prefix"].startswith("Document: docs/fixture.md > Install")
assert all(not (c["text"].strip().startswith("#") and len([l for l in c["text"].splitlines() if l.strip()]) == 1) for c in chunks)
PY

    cp "$WORK/index.json" "$WORK/index.first.json"
    run bash "$SCRIPT" ingest-index --config "$WORK/config.json" --root "$WORK" --out "$WORK/index.second.json"
    [ "$status" -eq 0 ]
    cmp "$WORK/index.first.json" "$WORK/index.second.json"

    python3 - "$WORK/config.json" <<'PY'
import json, sys
p=sys.argv[1]
d=json.load(open(p))
d["chunking"]["contextual_prefix"] = False
json.dump(d, open(p,"w"), sort_keys=True)
PY
    run bash "$SCRIPT" ingest-index --config "$WORK/config.json" --root "$WORK" --out "$WORK/index.changed.json"
    [ "$status" -eq 0 ]
    python3 - "$WORK/index.first.json" "$WORK/index.changed.json" <<'PY'
import json, sys
a=json.load(open(sys.argv[1])); b=json.load(open(sys.argv[2]))
assert a["chunking"]["chunk_config_hash"] != b["chunking"]["chunk_config_hash"]
assert a["index_version"] != b["index_version"]
assert not b["chunks"][0].get("contextual_prefix")
PY
}

@test "retrieve-eval: hybrid BM25+dense RRF reranking filters metadata and beats dense baseline" {
    cat > "$WORK/config.json" <<'JSON'
{
  "corpus": {"include": ["docs/**/*.md"]},
  "chunking": {"strategy": "structure_aware", "late_chunking": false, "contextual_prefix": false, "chunk_size": 900, "overlap": 0},
  "embedding": {"model_id": "local-test-embed", "index_version_prefix": "rag-index-v1"},
  "retrieval": {"dense_top_k": 4, "bm25_top_k": 4, "rrf_k": 60, "fusion_weight": 0.50, "rerank_top_n": 4, "final_n": 2},
  "query_transform": {"rewrite": false, "hyde": false, "multi_query": false},
  "freshness": {"serve_latest": true, "invalidate_by": ["doc_id", "content_hash"]},
  "eval": {"target_metric": "ndcg", "ragas_faithfulness_floor": 0.90}
}
JSON
    cat > "$WORK/index.json" <<'JSON'
{
  "schema_version": 1,
  "index_version": "rag-index-v1:test",
  "chunks": [
    {"chunk_id":"dense-decoy","doc_id":"guide-v2","source_path":"docs/guide.md","heading":"Semantic search","text":"Semantic retrieval uses vector meaning for broad conceptual matching in the documentation search layer.","metadata":{"doc_version":"v2","section":"search","product_area":"docs"}},
    {"chunk_id":"rrf-target","doc_id":"rag-v2","source_path":"docs/rag.md","heading":"Hybrid retrieval","text":"RRF combines dense vector results with BM25 exact-term matches for jargon such as rrf_k and Reciprocal Rank Fusion.","metadata":{"doc_version":"v2","section":"search","product_area":"docs"}},
    {"chunk_id":"old-version","doc_id":"rag-v1","source_path":"docs/rag-old.md","heading":"Old hybrid retrieval","text":"RRF legacy notes mention rrf_k but belong to the prior product version.","metadata":{"doc_version":"v1","section":"search","product_area":"docs"}},
    {"chunk_id":"unrelated","doc_id":"ops-v2","source_path":"docs/ops.md","heading":"Operations","text":"Deployment runbooks describe rollout windows and incident response.","metadata":{"doc_version":"v2","section":"ops","product_area":"platform"}}
  ]
}
JSON
    cat > "$WORK/golden.json" <<'JSON'
{
  "queries": [
    {"query":"rrf_k", "relevant":{"rrf-target":3}, "filters":{"doc_version":"v2", "section":"search"}},
    {"query":"semantic vector documentation search", "relevant":{"dense-decoy":2}, "filters":{"doc_version":"v2", "product_area":"docs"}}
  ]
}
JSON

    run bash "$SCRIPT" retrieve --index "$WORK/index.json" --config "$WORK/config.json" --query "rrf_k" --filter doc_version=v2 --filter section=search
    [ "$status" -eq 0 ]
    python3 - "$output" <<'PY'
import json, sys
out=json.loads(sys.argv[1])
ids=[r["chunk_id"] for r in out["results"]]
assert ids[0] == "rrf-target", ids
assert "old-version" not in ids, ids
assert out["knobs"]["fusion_weight"] == 0.5
assert out["stages"]["reranked"] <= out["knobs"]["rerank_top_n"]
PY

    run bash "$SCRIPT" retrieve-eval --index "$WORK/index.json" --config "$WORK/config.json" --golden "$WORK/golden.json" --mode dense --final-n 2
    [ "$status" -eq 0 ]
    dense_json="$output"
    run bash "$SCRIPT" retrieve-eval --index "$WORK/index.json" --config "$WORK/config.json" --golden "$WORK/golden.json" --mode hybrid --final-n 2
    [ "$status" -eq 0 ]
    hybrid_json="$output"
    python3 - "$dense_json" "$hybrid_json" <<'PY'
import json, sys
base=json.loads(sys.argv[1])
hybrid=json.loads(sys.argv[2])
assert hybrid["retrieval"]["ndcg"] > base["retrieval"]["ndcg"], (base, hybrid)
assert hybrid["retrieval"]["mrr"] >= base["retrieval"]["mrr"], (base, hybrid)
assert hybrid["retrieval"]["queries"] == 2
PY
}

@test "query-router-eval: routes transforms per query class and disables net-negative transforms" {
    cat > "$WORK/config.json" <<'JSON'
{
  "corpus": {"include": ["docs/**/*.md"]},
  "chunking": {"strategy": "structure_aware", "late_chunking": false, "contextual_prefix": false, "chunk_size": 900, "overlap": 0},
  "embedding": {"model_id": "local-test-embed", "index_version_prefix": "rag-index-v1"},
  "retrieval": {"dense_top_k": 6, "bm25_top_k": 6, "rrf_k": 60, "fusion_weight": 0.50, "rerank_top_n": 6, "final_n": 2},
  "query_transform": {
    "rewrite": "routed",
    "hyde": "routed",
    "multi_query": "routed",
    "decomposition": "routed",
    "max_added_tokens": 80,
    "min_recall_lift": 0.01,
    "rewrite_expansions": {"rrf_k": "reciprocal rank fusion constant rrf_k"}
  },
  "freshness": {"serve_latest": true, "invalidate_by": ["doc_id", "content_hash"]},
  "eval": {"target_metric": "ndcg", "ragas_faithfulness_floor": 0.90}
}
JSON
    cat > "$WORK/index.json" <<'JSON'
{
  "schema_version": 1,
  "index_version": "rag-index-v1:test",
  "chunks": [
    {"chunk_id":"easy-target","doc_id":"retrieval","source_path":"docs/retrieval.md","heading":"Hybrid retrieval","text":"Hybrid retrieval combines dense vector search with BM25 keyword search for documentation.","metadata":{"doc_version":"v2","section":"search"}},
    {"chunk_id":"sparse-target","doc_id":"rrf","source_path":"docs/rrf.md","heading":"Reciprocal Rank Fusion","text":"The reciprocal rank fusion constant controls how dense and sparse rankings are fused.","metadata":{"doc_version":"v2","section":"search"}},
    {"chunk_id":"hyde-decoy","doc_id":"hyde","source_path":"docs/hyde.md","heading":"HyDE rrf_k","text":"Hypothetical documents add generic implementation details and can distract exact rrf_k lookups.","metadata":{"doc_version":"v2","section":"search"}},
    {"chunk_id":"rrfk-decoy","doc_id":"legacy","source_path":"docs/legacy.md","heading":"Legacy rrf_k notes","text":"Legacy rrf_k notes mention an obsolete tuning knob only.","metadata":{"doc_version":"v2","section":"search"}},
    {"chunk_id":"setup-decoy","doc_id":"setup-overview","source_path":"docs/setup-overview.md","heading":"Bootstrap and AUTOSPEC_HOME overview","text":"A short overview mentions bootstrap and AUTOSPEC_HOME without install or configure details.","metadata":{"doc_version":"v2","section":"setup"}},
    {"chunk_id":"install-target","doc_id":"install","source_path":"docs/install.md","heading":"Install autospec","text":"Bootstrap autospec by running the install script before preparing the workspace.","metadata":{"doc_version":"v2","section":"setup"}},
    {"chunk_id":"configure-target","doc_id":"configure","source_path":"docs/configure.md","heading":"Configure AUTOSPEC_HOME","text":"Set AUTOSPEC_HOME so autospec stores run state in the expected directory.","metadata":{"doc_version":"v2","section":"setup"}}
  ]
}
JSON
    cat > "$WORK/golden.json" <<'JSON'
{
  "queries": [
    {"query":"hybrid retrieval dense bm25", "query_class":"easy", "relevant":{"easy-target":3}, "filters":{"doc_version":"v2", "section":"search"}},
    {"query":"rrf_k", "query_class":"sparse", "relevant":{"sparse-target":3}, "filters":{"doc_version":"v2", "section":"search"}},
    {"query":"bootstrap and AUTOSPEC_HOME", "query_class":"multi_hop", "relevant":{"install-target":2,"configure-target":2}, "filters":{"doc_version":"v2", "section":"setup"}}
  ]
}
JSON

    run bash "$SCRIPT" query-router-eval --index "$WORK/index.json" --config "$WORK/config.json" --golden "$WORK/golden.json"
    [ "$status" -eq 0 ]
    python3 - "$output" <<'PY'
import json, sys
out=json.loads(sys.argv[1])
assert out["retrieval"]["queries"] == 3, out
by_query={row["query"]: row for row in out["per_query"]}
easy=by_query["hybrid retrieval dense bm25"]
assert easy["query_class"] == "easy", easy
assert easy["enabled_transforms"] == [], easy
assert easy["decisions"]["rewrite"]["enabled"] is False, easy
sparse=by_query["rrf_k"]
assert sparse["query_class"] == "sparse", sparse
assert "rewrite" in sparse["enabled_transforms"], sparse
assert sparse["decisions"]["rewrite"]["recall_lift"] > 0, sparse
assert sparse["decisions"]["hyde"]["enabled"] is False, sparse
assert sparse["decisions"]["hyde"]["reason"] == "net_negative", sparse
assert sparse["decisions"]["hyde"]["added_tokens"] > 0, sparse
multi=by_query["bootstrap and AUTOSPEC_HOME"]
assert multi["query_class"] == "multi_hop", multi
assert "decomposition" in multi["enabled_transforms"], multi
assert multi["decisions"]["decomposition"]["recall_lift"] > 0, multi
assert multi["metrics"]["recall_at_k"] > multi["baseline_metrics"]["recall_at_k"], multi
assert set(multi["results"][:2]) == {"install-target", "configure-target"}, multi
PY
}

@test "eval-golden: versioned golden set emits deterministic retrieval, held-out, and RAGAS contract scores" {
    cat > "$WORK/config.json" <<'JSON'
{
  "corpus": {"include": ["docs/**/*.md"]},
  "chunking": {"strategy": "structure_aware", "late_chunking": false, "contextual_prefix": false, "chunk_size": 900, "overlap": 0},
  "embedding": {"model_id": "local-test-embed", "index_version_prefix": "rag-index-v1"},
  "retrieval": {"dense_top_k": 4, "bm25_top_k": 4, "rrf_k": 60, "fusion_weight": 0.50, "rerank_top_n": 4, "final_n": 2},
  "query_transform": {"rewrite": false, "hyde": false, "multi_query": false},
  "freshness": {"serve_latest": true, "invalidate_by": ["doc_id", "content_hash"]},
  "eval": {"target_metric": "ndcg", "ragas_faithfulness_floor": 0.90, "heldout_fold": 1, "heldout_modulus": 4}
}
JSON
    cat > "$WORK/index.json" <<'JSON'
{"schema_version":1,"index_version":"rag-index-v1:test","chunks":[
  {"chunk_id":"install","doc_id":"install","source_path":"docs/install.md","heading":"Install","text":"Install autospec with the bootstrap script before running agents.","metadata":{"section":"setup"}},
  {"chunk_id":"configure","doc_id":"configure","source_path":"docs/configure.md","heading":"Configure","text":"Set AUTOSPEC_HOME so run state and process heartbeats are stored consistently.","metadata":{"section":"setup"}},
  {"chunk_id":"rrf","doc_id":"rrf","source_path":"docs/rag.md","heading":"RRF","text":"Hybrid retrieval combines BM25 and dense rankings with reciprocal rank fusion.","metadata":{"section":"search"}}
]}
JSON
    cat > "$WORK/golden.json" <<'JSON'
{"schema_version":1,"version":"test-golden-v1","heldout":{"fold":1,"modulus":4},"queries":[
  {"id":"q1","question":"How do I install autospec?","ideal_answer":"Install autospec with the bootstrap script.","relevant_chunk_ids":["install"],"filters":{"section":"setup"}},
  {"id":"q2","question":"Where is AUTOSPEC_HOME used?","ideal_answer":"AUTOSPEC_HOME stores run state and process heartbeats.","relevant_chunk_ids":["configure"],"filters":{"section":"setup"}},
  {"id":"q3","question":"How does hybrid retrieval combine ranks?","ideal_answer":"Hybrid retrieval combines BM25 and dense rankings with reciprocal rank fusion.","relevant_chunk_ids":["rrf"],"filters":{"section":"search"}},
  {"id":"q4","question":"Which chunk mentions process heartbeats?","ideal_answer":"Process heartbeats are stored under AUTOSPEC_HOME.","relevant_chunk_ids":["configure"],"filters":{"section":"setup"}}
]}
JSON

    run bash "$SCRIPT" eval-golden --index "$WORK/index.json" --config "$WORK/config.json" --golden "$WORK/golden.json" --out "$WORK/eval.json"
    [ "$status" -eq 0 ]
    [[ "$output" == *"rag eval-golden wrote"* ]]
    python3 - "$WORK/eval.json" <<'PY'
import json, sys
out=json.load(open(sys.argv[1]))
assert out["golden_set"]["schema_version"] == 1, out
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

@test "auto-tune: records audit scores, promotes only positive nDCG, and quarantines faithfulness regression" {
    cat > "$WORK/base-config.json" <<'JSON'
{
  "corpus": {"include": ["docs/**/*.md"]},
  "chunking": {"strategy": "structure_aware", "late_chunking": false, "contextual_prefix": false, "chunk_size": 900, "overlap": 0},
  "embedding": {"model_id": "local-test-embed", "index_version_prefix": "rag-index-v1"},
  "retrieval": {"dense_top_k": 4, "bm25_top_k": 4, "rrf_k": 60, "fusion_weight": 1.0, "rerank_top_n": 4, "final_n": 1},
  "query_transform": {"rewrite": false, "hyde": false, "multi_query": false},
  "freshness": {"serve_latest": true, "invalidate_by": ["doc_id", "content_hash"]},
  "eval": {"target_metric": "ndcg", "ragas_faithfulness_floor": 0.90, "heldout_fold": 0, "heldout_modulus": 2}
}
JSON
    python3 - "$WORK/base-config.json" "$WORK/good-config.json" "$WORK/bad-config.json" <<'PY'
import json, sys
base=json.load(open(sys.argv[1]))
good=json.loads(json.dumps(base)); good["retrieval"]["fusion_weight"]=0.0; good["retrieval"]["final_n"]=2
bad=json.loads(json.dumps(good)); bad["ragas"]={"deterministic_faithfulness": 0.80}
for path, obj in [(sys.argv[2], good), (sys.argv[3], bad)]:
    json.dump(obj, open(path,"w"), indent=2, sort_keys=True)
PY
    cat > "$WORK/index.json" <<'JSON'
{"schema_version":1,"index_version":"rag-index-v1:test","chunks":[
  {"chunk_id":"dense-decoy","doc_id":"semantic","source_path":"docs/semantic.md","heading":"Semantic","text":"Semantic vector search handles broad documentation concepts.","metadata":{"section":"search"}},
  {"chunk_id":"rrf-target","doc_id":"rrf","source_path":"docs/rrf.md","heading":"RRF","text":"The rrf_k knob configures reciprocal rank fusion for exact BM25 jargon.","metadata":{"section":"search"}},
  {"chunk_id":"install-target","doc_id":"install","source_path":"docs/install.md","heading":"Install","text":"Install autospec with the bootstrap script.","metadata":{"section":"setup"}}
]}
JSON
    cat > "$WORK/golden.json" <<'JSON'
{"schema_version":1,"version":"tune-v1","queries":[
  {"id":"q1","question":"rrf_k","ideal_answer":"The rrf_k knob configures reciprocal rank fusion.","relevant_chunk_ids":{"rrf-target":3},"filters":{"section":"search"}},
  {"id":"q2","question":"install autospec","ideal_answer":"Install autospec with the bootstrap script.","relevant_chunk_ids":{"install-target":3},"filters":{"section":"setup"}}
]}
JSON

    run bash "$SCRIPT" auto-tune --index "$WORK/index.json" --config "$WORK/base-config.json" --golden "$WORK/golden.json" --candidate-config "$WORK/good-config.json" --audit-log "$WORK/audit.ndjson" --promote-out "$WORK/promoted.json" --quarantine-out "$WORK/quarantine.json"
    [ "$status" -eq 0 ]
    [[ "$output" == *"rag auto-tune promoted"* ]]
    [ -f "$WORK/promoted.json" ]
    [ ! -f "$WORK/quarantine.json" ]
    [ "$(wc -l < "$WORK/audit.ndjson" | tr -d ' ')" -eq 2 ]
    python3 - "$WORK/promoted.json" "$WORK/audit.ndjson" <<'PY'
import json, sys
prom=json.load(open(sys.argv[1]))
assert prom["decision"] == "promoted", prom
assert prom["candidate"]["retrieval"]["ndcg"] > prom["baseline"]["retrieval"]["ndcg"], prom
rows=[json.loads(line) for line in open(sys.argv[2])]
assert {r["role"] for r in rows} == {"baseline", "candidate"}, rows
assert all("config_version" in r and "retrieval" in r and "ragas" in r for r in rows), rows
PY

    rm -f "$WORK/promoted.json" "$WORK/quarantine.json" "$WORK/audit.ndjson"
    run bash "$SCRIPT" auto-tune --index "$WORK/index.json" --config "$WORK/base-config.json" --golden "$WORK/golden.json" --candidate-config "$WORK/bad-config.json" --audit-log "$WORK/audit.ndjson" --promote-out "$WORK/promoted.json" --quarantine-out "$WORK/quarantine.json"
    [ "$status" -eq 1 ]
    [[ "$output" == *"RAGAS_FAITHFULNESS_FLOOR"* ]]
    [[ "$output" == *"rag auto-tune quarantined"* ]]
    [ ! -f "$WORK/promoted.json" ]
    [ -f "$WORK/quarantine.json" ]
    python3 - "$WORK/quarantine.json" <<'PY'
import json, sys
q=json.load(open(sys.argv[1]))
assert q["decision"] == "quarantined", q
assert any(f.startswith("RAGAS_FAITHFULNESS_FLOOR") for f in q["findings"]), q
PY
}
