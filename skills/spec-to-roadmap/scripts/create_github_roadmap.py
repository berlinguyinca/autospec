#!/usr/bin/env python3
"""Create GitHub roadmap issues/project from a JSON plan.

This script intentionally handles the mechanical GitHub operations only.
Codex remains responsible for reading the spec and producing the plan.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path


def run(args: list[str], *, input_text: str | None = None) -> str:
    result = subprocess.run(
        args,
        input=input_text,
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


def gh_json(args: list[str]) -> object:
    out = run(["gh", *args, "--format", "json"])
    return json.loads(out)


def ensure_label(repo: str, label: dict[str, str]) -> None:
    name = label["name"]
    existing = run(["gh", "label", "list", "--repo", repo, "--limit", "300"])
    names = {line.split("\t", 1)[0] for line in existing.splitlines() if line}
    if name in names:
        return
    run(
        [
            "gh",
            "label",
            "create",
            name,
            "--repo",
            repo,
            "--color",
            label.get("color", "ededed"),
            "--description",
            label.get("description", ""),
        ]
    )


def create_issue(repo: str, title: str, labels: list[str], body: str) -> tuple[int, str]:
    args = ["gh", "issue", "create", "--repo", repo, "--title", title, "--body", body]
    if labels:
        args.extend(["--label", ",".join(labels)])
    url = run(args).splitlines()[-1]
    return int(url.rstrip("/").split("/")[-1]), url


def edit_issue_body(repo: str, number: int, body: str) -> None:
    run(["gh", "issue", "edit", str(number), "--repo", repo, "--body", body])


def create_project(owner: str, title: str) -> tuple[int, str]:
    data = gh_json(["project", "create", "--owner", owner, "--title", title])
    return int(data["number"]), data["url"]


def edit_project(owner: str, number: int, project: dict[str, str]) -> None:
    args = ["gh", "project", "edit", str(number), "--owner", owner]
    if project.get("description"):
        args.extend(["--description", project["description"]])
    if project.get("readme"):
        args.extend(["--readme", project["readme"]])
    if len(args) > 5:
        run(args)


def add_project_item(owner: str, project_number: int, url: str) -> None:
    run(["gh", "project", "item-add", str(project_number), "--owner", owner, "--url", url])


def render(template: str, values: dict[str, str]) -> str:
    for key, value in values.items():
        template = template.replace("{{" + key + "}}", value)
    return template


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("plan", type=Path)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    plan = json.loads(args.plan.read_text())
    repo = plan["repo"]
    project = plan["project"]
    owner = project["owner"]

    if args.dry_run:
        print(json.dumps(plan, indent=2))
        return 0

    for label in plan.get("labels", []):
        ensure_label(repo, label)

    tracker_num, tracker_url = create_issue(
        repo,
        plan["tracker"]["title"],
        plan["tracker"].get("labels", []),
        plan["tracker"]["body"],
    )

    issue_refs: dict[str, str] = {}
    child_lines: list[str] = []
    child_urls: list[str] = []
    for issue in plan["issues"]:
        number, url = create_issue(repo, issue["title"], issue.get("labels", []), issue["body"])
        issue_refs[issue["key"]] = f"#{number}"
        child_lines.append(f"- #{number} {issue['title']}")
        child_urls.append(url)
        print(f"created #{number}: {issue['title']}")

    child_list = "\n".join(child_lines)
    tracker_body = render(
        plan["tracker"]["body"],
        {"child_list": child_list, "tracker_ref": f"#{tracker_num}", "tracker_url": tracker_url},
    )
    edit_issue_body(repo, tracker_num, tracker_body)

    project_number, project_url = create_project(owner, project["title"])
    edit_project(owner, project_number, project)

    source_spec = str(plan.get("source_spec", ""))
    if source_spec.startswith("#"):
        source_url = f"https://github.com/{repo}/issues/{source_spec[1:]}"
        add_project_item(owner, project_number, source_url)
    add_project_item(owner, project_number, tracker_url)
    for url in child_urls:
        add_project_item(owner, project_number, url)

    comment = plan.get("source_comment")
    if comment and source_spec.startswith("#"):
        body = render(
            comment,
            {
                "project_url": project_url,
                "tracker_ref": f"#{tracker_num}",
                "tracker_url": tracker_url,
                "child_list": child_list,
            },
        )
        run(["gh", "issue", "comment", source_spec[1:], "--repo", repo, "--body", body])

    print(f"project: {project_url}")
    print(f"tracker: #{tracker_num} {tracker_url}")
    print(child_list)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        raise SystemExit(1)
