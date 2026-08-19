#!/usr/bin/env python3
"""Turn model-artifacts.yaml into a fetch plan.

model-artifacts.yaml already records provenance, so it is the single source for
what to download too -- a new model is then added in one place rather than two.

    artifacts.py --plan config/model-artifacts.yaml

prints one tab-separated line per file: file, repository, revision, size_bytes.
Consumed by install-node.sh's weights phase.

Revisions, never branches: the 27B repository was modified on the same day these
weights were first fetched, so a branch download is already irreproducible.
"""
from __future__ import annotations

import argparse
import sys

try:
    import yaml
except ImportError:  # pragma: no cover
    yaml = None


def artifact_fetch_plan(yaml_text: str) -> list[dict]:
    """Flatten artifacts and their projectors into one list of files to fetch."""
    if yaml is None:
        raise SystemExit("pyyaml is required to read model-artifacts.yaml")
    doc = yaml.safe_load(yaml_text) or {}
    out: list[dict] = []
    for a in (doc.get("artifacts") or []):
        if not isinstance(a, dict):
            continue
        repo = a.get("repository")
        rev = a.get("revision")
        # The weights themselves.
        if a.get("file") and repo and rev and a.get("size_bytes"):
            out.append({"file": a["file"], "repository": repo,
                        "revision": str(rev), "size_bytes": int(a["size_bytes"])})
        # An optional projector, which may live in the same repository or another.
        proj = a.get("projector")
        if isinstance(proj, dict) and proj.get("file") and proj.get("size_bytes"):
            out.append({
                "file": proj["file"],
                "repository": proj.get("repository") or repo,
                "revision": str(proj.get("revision") or rev),
                "size_bytes": int(proj["size_bytes"]),
            })
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--plan", required=True)
    args = ap.parse_args()
    for e in artifact_fetch_plan(open(args.plan).read()):
        print("%s\t%s\t%s\t%d" % (e["file"], e["repository"], e["revision"],
                                  e["size_bytes"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
