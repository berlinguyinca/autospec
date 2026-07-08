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
  "retrieval": {"dense_top_k": 50, "bm25_top_k": 50, "rrf_k": 60, "rerank_top_n": 50, "final_n": 8},
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
  "retrieval": {"dense_top_k": 50, "bm25_top_k": 50, "rrf_k": 60, "rerank_top_n": 50, "final_n": 8},
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
