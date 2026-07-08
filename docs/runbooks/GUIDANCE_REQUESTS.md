# Guidance Requests

Guidance requests capture human decisions needed for stuck, medium-risk, or high-risk work.

```bash
bash scripts/autospec-build-guidance-request.sh --dry-run --issue <number>
bash scripts/autospec-build-guidance-request.sh --confirm --issue <number>
```

Confirmed mode writes local guidance state. It does not auto-resume work.
