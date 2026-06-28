# AI/NLAI Simulation

AI/NLAI simulation is mock-only by default and never calls OpenAI, Ollama, MCP servers, or external providers.

```bash
bash scripts/autospec-simulate-ai-nlai.sh --dry-run --scenario rag-docs
bash scripts/autospec-simulate-ai-nlai.sh --confirm --mock-only --scenario no-context-fallback
bash scripts/autospec-token-usage-evidence.sh --dry-run
```

Simulation checks shell/spec existence, secret display risk, no-context fallback, citation/source areas, token usage planning, pretty rendering, and raw JSON avoidance.
