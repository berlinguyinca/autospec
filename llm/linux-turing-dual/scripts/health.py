#!/usr/bin/env python3
"""What is wrong with this node and its fleet, said plainly.

Written because the node once reported itself healthy while it could not serve.
Both of its GPUs took an `Xid 154`, llama.cpp failed to initialise CUDA, and six
of seven models became black holes -- and the dashboard showed `online`, seven
models, because liveness was measured at the PROCESS level. The router process was
alive. It simply could not produce a token.

Every check here turns a signal the node already collects into a sentence a person
can act on. Three rules hold throughout:

  * A problem names what it MEANS, not what emitted it. "a GPU is not responding"
    is actionable; "nvidia-smi exit 6" sends the reader to a search engine.
  * Severity is about capability, not tone. `down` means requests cannot be
    served; `degraded` means they can, with less than the machine should have;
    `warning` means something needs attention but nothing is lost yet.
  * A signal that could not be READ is never reported as a clean result. An
    unreadable journal is its own problem, because the alternative is silence
    that looks like health -- which is the exact failure this module exists for.
"""
from __future__ import annotations

import re

DOWN, DEGRADED, WARNING = "down", "degraded", "warning"

# Driver-level faults, from nvidia-smi's stderr. It exits ZERO while printing
# these, having answered for the cards it could still reach -- so the text is the
# only signal, and the old collector discarded it.
_GPU_GONE = (
    ("Unable to determine the device handle",
     "a GPU is not responding to the driver"),
    ("has fallen off the bus", "a GPU has fallen off the bus"),
    ("insufficient permissions", "this node may not read its own GPUs"),
)
# Checked only when nothing above matched. The real line from this node's outage
# was `Unable to determine the device handle for GPU1: 0000:44:00.0: Unknown
# Error` -- it matches a specific pattern AND this one, and reporting both made a
# single dead card read as two faults.
_GPU_VAGUE = (
    ("Unknown Error", "the driver reported an unknown error on a GPU"),
    ("ERR!", "the driver could not read a GPU's sensors"),
)

# Runtime faults, from the router's journal. `Xid` is the driver telling the
# kernel a GPU faulted; llama.cpp's CUDA messages are it discovering the same
# thing from the other side.
_JOURNAL_FAULTS = (
    (re.compile(r"Xid \(", re.I),
     "the driver logged a GPU fault (Xid) -- this usually needs a reboot", DOWN),
    (re.compile(r"failed to initialize CUDA", re.I),
     "the runtime could not initialise CUDA, so it cannot use the GPUs", DOWN),
    (re.compile(r"ggml_cuda_error|CUDA error", re.I),
     "the runtime hit a CUDA error while serving", DOWN),
    # NOT case-insensitive `OOM`: that matched "to make rOOM for" in the
    # router's own healthy eviction line and reported a memory failure on a node
    # that had none. The acronym is upper-case and a word; the phrase is a
    # phrase. A detector's false positive is worse than a missing check, because
    # it sends someone to fix the wrong thing.
    # The PHRASE is case-insensitive (the kernel capitalises "Out of memory");
    # the ACRONYM is not, and must be a whole word. Case-insensitive `OOM` matched
    # "to make rOOM for" -- the router's own healthy eviction line -- and reported
    # a memory failure on a node that had none. A detector's false positive is
    # worse than a missing check: it sends someone to fix the wrong thing.
    (re.compile(r"(?i:out of memory|failed to allocate)|\bOOM\b|oom-kill"),
     "the runtime ran out of memory", DEGRADED),
)


def _p(severity: str, text: str, where: str = "this node") -> dict:
    return {"severity": severity, "text": text, "where": where}


def gpu_problems(smi_stderr: str, cards, smi_failed: bool = False) -> list[dict]:
    """Driver-level faults, and the case where nvidia-smi could not be run."""
    out = []
    if smi_failed:
        # Distinct from "no cards": one means the question could not be asked.
        out.append(_p(WARNING, "the GPU tool could not be run, so card "
                               "telemetry is unavailable"))
    err = (smi_stderr or "").lower()
    seen = set()
    for needle, text in _GPU_GONE:
        if needle.lower() in err and text not in seen:
            seen.add(text)
            out.append(_p(DOWN, text))
    if not seen:
        for needle, text in _GPU_VAGUE:
            if needle.lower() in err and text not in seen:
                seen.add(text)
                out.append(_p(DOWN, text))
    if not smi_failed and not cards and not out:
        out.append(_p(DOWN, "no GPU is visible to this node"))
    return out


def runtime_problems(llama_up: bool, journal: str,
                     journal_readable: bool = True) -> list[dict]:
    """Faults the runtime reported about itself."""
    out = []
    if not llama_up:
        out.append(_p(DOWN, "the inference runtime is not answering"))
    if not journal_readable:
        out.append(_p(WARNING, "this node's log could not be read, so runtime "
                               "faults would go unnoticed"))
        return out
    for pattern, text, severity in _JOURNAL_FAULTS:
        if pattern.search(journal or ""):
            out.append(_p(severity, text))
    return out


def server_problems(row: dict) -> list[dict]:
    """Faults of one FLEET MEMBER, from what the node knows about it.

    Never quotes the server's own error string: those name hosts and ports, and
    this list is public. The category is reported instead.
    """
    sid = row.get("id") or "a server"
    state = row.get("state")
    out = []
    if state == "offline":
        out.append(_p(DOWN, "not answering", sid))
    elif state == "unknown":
        out.append(_p(WARNING, "registered but not yet seen", sid))
    elif not (row.get("models") or []):
        # The shape of today's failure, seen from the other side: attached, and
        # able to serve nothing.
        out.append(_p(DOWN, "online but reporting no models, so nothing can be "
                            "routed to it", sid))
    if row.get("error"):
        out.append(_p(WARNING, "the last probe of it failed", sid))
    if state == "online" and row.get("kind") == "tunnel" \
            and row.get("idle_pipes") == 0:
        out.append(_p(WARNING, "attached with no spare connection, so a request "
                               "may have to wait", sid))
    if row.get("enabled") is False:
        out.append(_p(WARNING, "disabled, so the balancer will not use it", sid))
    # Configuration faults the registry already found ("no usable base_url" and
    # friends). Folded in here so the panel has ONE list to render: two lists of
    # problems side by side is how one of them stops being read.
    for text in row.get("problems") or []:
        out.append(_p(WARNING, f"configuration: {text}", sid))
    return out


def orphaned_models(advertised, servers) -> list[dict]:
    """Models nothing healthy can serve, which is what a caller actually feels.

    This is the check that would have caught the failure this module was written
    for: six models stayed advertised by a node whose runtime was dead, and every
    request for one of them hung until the client gave up.
    """
    healthy = set()
    for s in servers or []:
        # `faults` when the caller has already computed them -- which is how a
        # node's OWN problems (a dead GPU, a runtime that cannot initialise CUDA)
        # count here. Deriving them again from the row would miss those: they come
        # from a journal, not from anything visible in the row.
        faults = s.get("faults")
        if faults is None:
            faults = server_problems(s)
        if s.get("state") == "online" and not any(
                p.get("severity") == DOWN for p in faults):
            healthy.update(s.get("models") or [])
    lost = sorted(set(advertised or []) - healthy)
    if not lost:
        return []
    shown = ", ".join(lost[:4]) + ("…" if len(lost) > 4 else "")
    return [_p(DOWN, f"{len(lost)} advertised model(s) have no healthy server: "
                     f"{shown}", "the fleet")]


def worst(problems) -> str | None:
    """The severity to show at a glance. None when there is nothing wrong."""
    for level in (DOWN, DEGRADED, WARNING):
        if any(p.get("severity") == level for p in problems or []):
            return level
    return None
