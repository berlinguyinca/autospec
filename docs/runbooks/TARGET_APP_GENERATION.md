# Target App Generation

Autospec target-app generation uses runtime adapters and feature slices. Generation requires high stack confidence and an adapter match. Low-confidence stacks receive specs/issues instead of runtime files.

Safe default:

```yaml
autonomy:
  worker:
    allow_runtime_features: false
    runtime_feature_mode: shell_only
    require_stack_confidence: 0.8
    allow_partial_runtime: true
    allow_complete_runtime: false
```

Operators enable runtime generation explicitly by invoking the runtime generator or worker with `--feature`.
