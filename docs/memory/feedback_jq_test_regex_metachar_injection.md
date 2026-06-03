---
name: feedback-jq-test-regex-metachar-injection
description: "Interpolating host/user-derived values into a jq test() regex is a metachar-injection bug; autospec claim self-clean matched worker_id this way and deleted the wrong worker's lock comment for dotted hostnames"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 28ec6f0b-02b3-4a16-ac53-f4d2d5c2bc46
---

In autospec-run's cross-machine claim path (`skills/autospec-run/scripts/claim-issue.sh`), `own_marked_comment_id()` matched this worker's lock comment by interpolating `worker_id` into a jq `test("...worker_id...:\""+$wid+"\"")` **regex**. Worker ids are `host:user:monitor:pid`, and real hostnames contain `.` (`mac.lan`, `*.local`, FQDNs) — a regex metachar — so `mac.lan:...` false-matched `macXlan:...` and a losing worker DELETED THE WRONG worker's lock comment, reintroducing the duplicate-claim bug the whole feature fixes. Fixed in PR #879 (#876) by extracting the field via jq `capture()` and comparing with `==` literal equality.

**Why:** any value derived from hostnames/usernames/paths injected into `test()`/`match()` is regex-interpreted — dots, `+`, `*`, `[]`, `()` all break exact matching. The owner-arbitration path was already safe because it used `capture()`+fixed pattern; only the self-clean targeting used `test()`.

**How to apply:** in autospec claim/run-state code (and any jq), never put externally-derived strings inside `test()`/`match()` for equality — use `capture()` to pull the field, then `==`. The Phase 5.5 integration audit MUST keep checking worker_id/identity matching for this. Confirms [[feedback_per_pr_lgtm_misses_integration]] — per-PR LGTM passed all 4 PRs; only the broad audit caught it.
