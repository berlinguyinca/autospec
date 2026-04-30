#!/usr/bin/env python3
"""Generate a JSON roadmap-plan scaffold for a repo-local spec file.

The scaffold is intentionally incomplete: Codex still reads the spec and fills
the tracker plus child issue bodies. This script removes repetitive metadata and
keeps the plan shape consistent before `create_github_roadmap.py --dry-run`.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
from pathlib import Path


def run(args: list[str]) -> str:
    result = subprocess.run(
        args,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed ({result.returncode}): {' '.join(args)}\n{result.stderr}"
        )
    return result.stdout.strip()


def slugify(value: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", value.lower()).strip("-")
    return slug or "roadmap"


def default_branch(repo: str) -> str:
    try:
        data = json.loads(run(["gh", "repo", "view", repo, "--json", "defaultBranchRef"]))
        return str(data["defaultBranchRef"]["name"])
    except Exception:
        return "main"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--spec", required=True, help="Repo-relative spec path")
    parser.add_argument("--repo", required=True, help="GitHub repo as owner/name")
    parser.add_argument("--project-owner", required=True)
    parser.add_argument("--project-title", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--source-url", help="Override committed spec URL")
    parser.add_argument("--global-gate", action="append", default=[])
    args = parser.parse_args()

    spec_path = args.spec.strip()
    branch = default_branch(args.repo)
    source_url = args.source_url or f"https://github.com/{args.repo}/blob/{branch}/{spec_path}"
    project_slug = slugify(args.project_title)

    plan = {
        "repo": args.repo,
        "source_spec": spec_path,
        "source_spec_paths": [spec_path],
        "source_spec_url": source_url,
        "project": {
            "owner": args.project_owner,
            "number": None,
            "title": args.project_title,
            "reuse_existing": True,
            "description": "",
            "readme": "",
        },
        "global_gates": args.global_gate,
        "existing_issue_links": [],
        "labels": [
            {
                "name": f"spec:{project_slug}",
                "color": "5319E7",
                "description": f"Derived from {args.project_title}",
            }
        ],
        "tracker": {
            "key": "tracker",
            "title": f"[{args.project_title}] Tracker: implementation roadmap",
            "labels": [f"spec:{project_slug}", "type:tracker"],
            "idempotency_key": f"{project_slug}-tracker",
            "body": (
                "## Goal\n\n...\n\n"
                "## Source spec\n\n- {{source_spec}}\n- {{source_spec_url}}\n\n"
                "## Roadmap\n\n{{child_list}}\n"
            ),
        },
        "issues": [],
        "source_comment": "",
    }

    args.output.write_text(json.dumps(plan, indent=2) + "\n", encoding="utf-8")
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
