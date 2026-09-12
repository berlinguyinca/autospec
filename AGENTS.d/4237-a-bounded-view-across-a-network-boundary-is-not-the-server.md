# A bounded view across a network boundary is not the server (issue #4237)

One client's log. The `bender` workstation runs an agent that offers its GPU to
a public inference service, and that agent's journal showed:

```
attaches:  Sep 03: 19   Sep 04-06: none
404s:      Sep 07: 396  Sep 08: 1346  Sep 09: 1349  Sep 10: 881
```

From that, the service was diagnosed as having "no inference capacity for
three days". The operator replied it had been serving normally thirty minutes
earlier. They were right, and they had a measurement; I had exactly one
client-side view. What I never obtained was any observation of the *service*
itself: `/v1/models` returns 401 without a credential, and I correctly declined
to send one to an external host. So I had client-side evidence and no
server-side evidence, and I wrote the conclusion as though I had both.

**"This client cannot connect" does not imply "no client can connect."** The
service had capacity from somewhere else the whole time.

- **A diagnosis of a remote service requires evidence from that service.**
  Client logs establish what the client experienced. They cannot establish the
  server's state, because the client sees exactly one of the server's
  relationships. Where the server cannot be observed, the honest output is
  "I cannot see it" — not an inference wearing a finding's clothes.
- **Say which side of the boundary each fact came from.** Had the issue read
  "from bender's journal: ..." and "from the service: nothing, /v1/models is
  401 and I did not authenticate", the gap would have been obvious to any
  reader, including the writer. The defect was not the missing access; it was
  presenting a one-sided view without labelling it as one.
- **An operator's direct observation outranks a derived one.** "It was
  serving 30 minutes ago" is a measurement of the thing itself. When it
  contradicts a chain of inference, the inference is what breaks — and the
  correction should be immediate and in the issue, because a filed issue in
  this pipeline gets implemented (cf. #4167).
- **Absence of evidence from a source you cannot query is not evidence of
  absence.** The same shape as reading `head -6` of a file and asserting what
  the file lacks (#4167), and listing one directory and asserting what the
  system does not have (#4192) — here at the level of a network boundary.
  Third instance, same shape: a bounded view treated as a complete one.

What the correction must not discard: the client genuinely cannot attach, and
for a real, findable reason. The service's live route table matches
`inferweave-gateway`, and that codebase implements no agent-control or
websocket-upgrade endpoint at all — eleven candidate paths all return 404. One
healthy GPU has been idle for a week, and that part of the finding stands.
Correcting an over-claim must not discard the part that was established.
