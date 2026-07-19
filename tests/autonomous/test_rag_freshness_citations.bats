#!/usr/bin/env bats
# tests/rag-freshness-citations.bats — issue #1552 RAG freshness and citation integrity.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/rag-workstream.sh"
    WORK="$(mktemp -d -t rag-freshness.XXXXXX)"
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

@test "freshness: incremental reindex keeps prior versions, rejects stale top hits, and verifies claim citations" {
    mkdir -p "$WORK/docs"
    cat > "$WORK/docs/a.md" <<'MD'
# Alpha
Alpha v1 explains retained chunks.
MD
    cat > "$WORK/docs/b.md" <<'MD'
# Beta
Beta v1 stays untouched.
MD
    cat > "$WORK/config.json" <<'JSON'
{
  "corpus": {"include": ["docs/**/*.md"]},
  "chunking": {"strategy": "structure_aware", "late_chunking": false, "contextual_prefix": false, "chunk_size": 900, "overlap": 0},
  "embedding": {"model_id": "local-test-embed", "index_version_prefix": "rag-index-v1"},
  "retrieval": {"dense_top_k": 8, "bm25_top_k": 8, "rrf_k": 60, "fusion_weight": 0.50, "rerank_top_n": 8, "final_n": 4},
  "query_transform": {"rewrite": false, "hyde": false, "multi_query": false},
  "freshness": {"serve_latest": true, "invalidate_by": ["doc_id", "content_hash"]},
  "eval": {"target_metric": "ndcg", "ragas_faithfulness_floor": 0.90}
}
JSON
    run bash "$SCRIPT" ingest-index --config "$WORK/config.json" --root "$WORK" --out "$WORK/index.v1.json"
    [ "$status" -eq 0 ]
    cp "$WORK/index.v1.json" "$WORK/index.v1.saved.json"

    cat > "$WORK/docs/a.md" <<'MD'
# Alpha
Alpha v2 explains fresh incremental chunks and citation support.
MD
    run bash "$SCRIPT" ingest-index --config "$WORK/config.json" --root "$WORK" --previous-index "$WORK/index.v1.json" --changed-doc docs/a.md --out "$WORK/index.v2.json"
    [ "$status" -eq 0 ]
    [[ "$output" == *"incremental=1"* ]]

    python3 - "$WORK/index.v1.saved.json" "$WORK/index.v2.json" <<'PY'
import json, sys
old=json.load(open(sys.argv[1])); new=json.load(open(sys.argv[2]))
old_b=[c for c in old['chunks'] if c['doc_id']=='docs/b.md']
new_b=[c for c in new['chunks'] if c['doc_id']=='docs/b.md']
assert old_b == new_b, (old_b, new_b)
new_a=[c for c in new['chunks'] if c['doc_id']=='docs/a.md']
assert len({c['doc_content_hash'] for c in new_a}) == 2, new_a
assert sorted({c['doc_version'] for c in new_a}) == [1, 2], new_a
assert [c for c in new_a if c.get('is_latest') and c['doc_version']==2], new_a
assert [c for c in new_a if (not c.get('is_latest')) and c['doc_version']==1], new_a
assert new['docs_by_id']['docs/a.md']['latest_version'] == 2, new['docs_by_id']['docs/a.md']
assert new['incremental_reindex']['changed_docs'] == ['docs/a.md'], new['incremental_reindex']
assert new['incremental_reindex']['reused_chunk_count'] == len(old_b), new['incremental_reindex']
assert new['incremental_reindex']['invalidated_doc_hashes']['docs/a.md'] == [old['docs_by_id']['docs/a.md']['latest_content_hash']], new['incremental_reindex']
PY

    run bash "$SCRIPT" retrieve --index "$WORK/index.v2.json" --config "$WORK/config.json" --query "retained chunks"
    [ "$status" -eq 0 ]
    python3 - "$output" <<'PY'
import json, sys
out=json.loads(sys.argv[1])
assert all(r['doc_id'] != 'docs/a.md' or r['metadata']['doc_version'] == '2' for r in out['results']), out
assert out['freshness']['status'] == 'ok', out
PY

    cat > "$WORK/stale-index.json" <<'JSON'
{"schema_version":1,"chunks":[{"chunk_id":"stale-a","doc_id":"docs/a.md","source_path":"docs/a.md","heading":"Alpha","text":"stale retained chunks answer","doc_content_hash":"stalehash","doc_version":1,"is_latest":true}]}
JSON
    run bash "$SCRIPT" retrieve --index "$WORK/stale-index.json" --config "$WORK/config.json" --root "$WORK" --query "stale retained chunks answer"
    [ "$status" -eq 0 ]
    python3 - "$output" <<'PY'
import json, sys
out=json.loads(sys.argv[1])
assert out['answer_status'] == 'rejected_stale_top_hit', out
assert out['freshness']['status'] == 'stale_top_hit', out
assert out['results'] == [], out
assert out['stale_results'][0]['chunk_id'] == 'stale-a', out
PY

    cat > "$WORK/claims.json" <<'JSON'
{"claims":[
  {"claim":"Alpha v2 explains fresh incremental chunks","chunk_id":"stale-a","supporting_span":"Alpha v2 explains fresh incremental chunks"},
  {"claim":"Alpha v2 explains Kubernetes deploys","chunk_id":"stale-a","supporting_span":"Alpha v2 explains Kubernetes deploys"}
]}
JSON
    python3 - "$WORK/index.v2.json" > "$WORK/chunks-for-citations.json" <<'PY'
import json, sys
idx=json.load(open(sys.argv[1]))
chunk=[c for c in idx['chunks'] if c['doc_id']=='docs/a.md' and c.get('is_latest')][0]
chunk['chunk_id']='stale-a'
json.dump({'chunks':[chunk]}, sys.stdout)
PY
    run bash "$SCRIPT" citation-check --claims "$WORK/claims.json" --chunks "$WORK/chunks-for-citations.json"
    [ "$status" -eq 1 ]
    [[ "$output" == *"UNSUPPORTED_CITATION:stale-a:Alpha v2 explains Kubernetes deploys"* ]]
}

@test "golden set includes a freshness canary question" {
    python3 - "$REPO_ROOT/.autospec/rag-workstream/golden-set.json" <<'PY'
import json, sys
queries=json.load(open(sys.argv[1]))['queries']
canaries=[q for q in queries if q.get('canary') == 'freshness']
assert canaries, queries
assert any('stale' in q['question'].lower() or 'fresh' in q['question'].lower() for q in canaries), canaries
PY
}
