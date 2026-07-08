# RAG documentation database workstream

Issue #1541 establishes the parent design gate for a continuous documentation RAG tier without swallowing the implementation scope of child issues #1548, #1549, #1550, #1551, and #1552.

## Parent contract

The workstream builds a reproducible index over `llms.txt`, `llms-full.txt`, and high-signal repository docs. The default config lives in `.autospec/rag-workstream/config.json`; `scripts/rag-workstream.sh config-version --config .autospec/rag-workstream/config.json` returns the index version as a hash of all knobs so changed chunking, embedding, retrieval, query-transform, freshness, or eval settings cannot silently mix embedding spaces.

The parent gate enforces these invariants before child issues fill in the concrete retrieval engine:

- Chunking is structure-aware, preserves Markdown heading/body and code-fence boundaries, and exposes late chunking plus a contextual prefix as knobs.
- Embeddings are versioned by `embedding.model_id` and the config hash; never mix vectors produced from different model/config spaces.
- Retrieval is always hybrid dense + BM25 with Reciprocal Rank Fusion, followed by reranking and metadata filters for index version, document version, section, and product area.
- Query transforms are routed knobs: rewrite, HyDE, multi-query, and decomposition run only for query classes where the golden set shows lift.
- Freshness uses doc-level content-hash invalidation, serves latest versions by default, and keeps prior versions filterable.
- The eval gate must promote only score-positive configs: target retrieval metric improvement plus no RAGAS faithfulness regression below the configured RAGAS faithfulness floor.
- Citation verification rejects claims that lack a cited chunk or whose supporting span is absent from the cited chunk.

## Deterministic gate commands

```bash
bash scripts/rag-workstream.sh config-version --config .autospec/rag-workstream/config.json
bash scripts/rag-workstream.sh ingest-index --config .autospec/rag-workstream/config.json --root . --out reports/rag/index.json
bash scripts/rag-workstream.sh gate --baseline reports/rag/baseline.json --candidate reports/rag/candidate.json --target-metric ndcg --faithfulness-floor 0.90 --promote-out reports/rag/promoted.json
bash scripts/rag-workstream.sh freshness-check --manifest reports/rag/index-manifest.json --root .
bash scripts/rag-workstream.sh chunk-boundary-check --chunks reports/rag/chunks.json
bash scripts/rag-workstream.sh citation-check --claims reports/rag/claims.json --chunks reports/rag/chunks.json
bash scripts/rag-workstream.sh validate-design-doc --doc docs/runbooks/rag-documentation-database.md
```

## Issue #1548 ingestion/index contract

`ingest-index` materializes a deterministic local JSON scaffold rather than calling a live embedding service. It walks the configured corpus (`llms.txt`, `llms-full.txt`, and Markdown docs by default), splits Markdown by headings outside code fences, keeps heading text attached to its body, and honors `chunking.chunk_size` / `chunking.overlap` when large sections need secondary block splitting.

Each emitted index carries `embedding_model_id`, `chunking.chunk_config_hash`, and `index_metadata.version_tuple`. The chunk hash is computed from the strategy, size/overlap, late-chunking toggle, contextual-prefix toggle, and boundary-preservation knobs; changing any of those values changes `index_version` and requires the downstream embedder to cleanly re-embed instead of appending into an old embedding space. When `chunking.contextual_prefix` is enabled, each chunk gets a cheap deterministic prefix in the style of Anthropic Contextual Retrieval; when `chunking.late_chunking` is enabled, the placeholder embedding input hash is derived from the full document to model Jina-style late chunking without adding external dependencies.

## Child issue handoff boundaries

- #1548 owns ingestion, structure-aware splitting, Jina late chunking, contextual-prefix generation, and materialized index writes.
- #1549 owns hybrid dense + BM25 retrieval, Reciprocal Rank Fusion scoring, rerank adapters, and metadata filtering.
- #1550 owns the query-transform router for rewrite, HyDE, multi-query, and multi-hop decomposition.
- #1551 owns the versioned golden set, retrieval metric computation, RAGAS nightly scoring, score ledger, and unattended tuning loop.
- #1552 owns incremental reindex, content-hash invalidation, latest-version serving, stale-index guards, and citation verification integrations.

## Source canon

- Anthropic Contextual Retrieval: https://www.anthropic.com/engineering/contextual-retrieval — cited for contextual prefixes, contextual BM25, and reported retrieval-failure reductions.
- RAGAS faithfulness metric: https://docs.ragas.io/en/stable/concepts/metrics/available_metrics/faithfulness/ — cited for checking whether answer claims are supported by retrieved context.
- RAGAS context precision metric: https://docs.ragas.io/en/stable/concepts/metrics/available_metrics/context_precision/ — cited for retriever ranking quality.
- Jina late-chunking: https://jina.ai/news/late-chunking-in-long-context-embedding-models/ — cited for producing contextual chunk embeddings after long-context embedding.
- llms.txt proposal: https://llmstxt.org/ — cited for using `llms.txt` as a machine-readable documentation corpus input.
