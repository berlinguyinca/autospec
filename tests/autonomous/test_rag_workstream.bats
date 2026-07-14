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

@test "direct Rust validation owns the RAG workstream bats suite and required artifacts" {
    catalog="$REPO_ROOT/crates/autospec-core/src/validation/catalog.rs"
    grep -q '"check_autonomous_phase2_suite"' "$catalog"
    grep -q 'BatsDirectory("tests/autonomous")' "$catalog"
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
