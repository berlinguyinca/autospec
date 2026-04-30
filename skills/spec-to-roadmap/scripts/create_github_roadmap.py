#!/usr/bin/env python3
"""Create or update GitHub roadmap issues/project from a JSON plan.

The script intentionally handles mechanical GitHub operations only. Codex is
still responsible for reading the spec, making engineering decisions, and
writing issue bodies that are implementation-ready.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any


REQUIRED_ISSUE_SECTIONS = (
    "## Goal",
    "## Source spec",
    "## Depends on",
    "## Unblocks",
    "## Implementation scope",
    "## Out of scope",
    "## Acceptance criteria",
    "## Verification",
)


@dataclass
class IssueRef:
    number: int
    url: str
    title: str
    created: bool

    @property
    def ref(self) -> str:
        return f"#{self.number}"


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


def gh_json(args: list[str]) -> Any:
    out = run(["gh", *args])
    return json.loads(out or "{}")


def validate_plan(plan: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    for field in ("repo", "source_spec", "project", "tracker", "issues"):
        if field not in plan:
            errors.append(f"missing top-level field: {field}")

    keys: set[str] = set()
    for issue in plan.get("issues", []):
        key = issue.get("key")
        if not key:
            errors.append(f"issue without key: {issue.get('title', '<missing title>')}")
            continue
        if key in keys:
            errors.append(f"duplicate issue key: {key}")
        keys.add(key)

    for issue in plan.get("issues", []):
        key = issue.get("key", "<missing>")
        body = issue.get("body", "")
        for section in REQUIRED_ISSUE_SECTIONS:
            if section not in body:
                errors.append(f"{key}: missing required section {section}")
        for dep in issue.get("depends_on", []):
            if isinstance(dep, str) and dep.startswith("#"):
                continue
            if dep not in keys:
                errors.append(f"{key}: unresolved depends_on key {dep}")
        for unblock in issue.get("unblocks", []):
            if isinstance(unblock, str) and unblock.startswith("#"):
                continue
            if unblock not in keys:
                errors.append(f"{key}: unresolved unblocks key {unblock}")
        if not issue.get("labels"):
            errors.append(f"{key}: missing labels")
        if not issue.get("parallelization"):
            errors.append(f"{key}: missing parallelization")

    project = plan.get("project", {})
    if not project.get("owner"):
        errors.append("project.owner is required")
    if not project.get("title") and not project.get("number"):
        errors.append("project.title or project.number is required")
    return errors


def render(template: str, values: dict[str, str], issue_refs: dict[str, IssueRef]) -> str:
    rendered = template
    for key, value in values.items():
        rendered = rendered.replace("{{" + key + "}}", value)
    for key, ref in issue_refs.items():
        rendered = rendered.replace("{{issue_ref:" + key + "}}", ref.ref)
        rendered = rendered.replace("{{issue_url:" + key + "}}", ref.url)
    return rendered


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


def find_issue_by_title(repo: str, title: str) -> IssueRef | None:
    data = gh_json(
        [
            "issue",
            "list",
            "--repo",
            repo,
            "--state",
            "all",
            "--search",
            f'in:title "{title}"',
            "--limit",
            "100",
            "--json",
            "number,title,url",
        ]
    )
    for issue in data:
        if issue.get("title") == title:
            return IssueRef(
                number=int(issue["number"]),
                url=str(issue["url"]),
                title=title,
                created=False,
            )
    return None


def upsert_issue(repo: str, title: str, labels: list[str], body: str) -> IssueRef:
    existing = find_issue_by_title(repo, title)
    if existing:
        run(["gh", "issue", "edit", str(existing.number), "--repo", repo, "--body", body])
        if labels:
            run(
                [
                    "gh",
                    "issue",
                    "edit",
                    str(existing.number),
                    "--repo",
                    repo,
                    "--add-label",
                    ",".join(labels),
                ]
            )
        return existing

    args = ["gh", "issue", "create", "--repo", repo, "--title", title, "--body", body]
    if labels:
        args.extend(["--label", ",".join(labels)])
    url = run(args).splitlines()[-1]
    return IssueRef(number=int(url.rstrip("/").split("/")[-1]), url=url, title=title, created=True)


def project_list(owner: str) -> list[dict[str, Any]]:
    data = gh_json(["project", "list", "--owner", owner, "--format", "json", "--limit", "100"])
    return list(data.get("projects", []))


def resolve_project(owner: str, project: dict[str, Any]) -> tuple[int, str]:
    if project.get("number"):
        number = int(project["number"])
        data = gh_json(["project", "view", str(number), "--owner", owner, "--format", "json"])
        return number, str(data["url"])

    title = str(project["title"])
    if project.get("reuse_existing", True):
        for existing in project_list(owner):
            if existing.get("title") == title:
                return int(existing["number"]), str(existing["url"])

    data = gh_json(["project", "create", "--owner", owner, "--title", title, "--format", "json"])
    return int(data["number"]), str(data["url"])


def edit_project(owner: str, number: int, project: dict[str, Any]) -> None:
    args = ["gh", "project", "edit", str(number), "--owner", owner]
    if project.get("description"):
        args.extend(["--description", project["description"]])
    if project.get("readme"):
        args.extend(["--readme", project["readme"]])
    if len(args) > 5:
        run(args)


def project_item_urls(owner: str, project_number: int) -> set[str]:
    data = gh_json(
        [
            "project",
            "item-list",
            str(project_number),
            "--owner",
            owner,
            "--format",
            "json",
            "--limit",
            "500",
        ]
    )
    urls: set[str] = set()
    for item in data.get("items", []):
        content = item.get("content") or {}
        url = content.get("url")
        if url:
            urls.add(str(url))
    return urls


def add_project_item_once(owner: str, project_number: int, url: str, existing_urls: set[str]) -> None:
    if url in existing_urls:
        return
    run(["gh", "project", "item-add", str(project_number), "--owner", owner, "--url", url])
    existing_urls.add(url)


def source_spec_url(repo: str, source_spec: str) -> str | None:
    if source_spec.startswith("#"):
        return f"https://github.com/{repo}/issues/{source_spec[1:]}"
    if source_spec.startswith("https://github.com/"):
        return source_spec
    return None


def child_list_lines(issue_refs: dict[str, IssueRef], issues: list[dict[str, Any]]) -> str:
    lines = []
    for issue in issues:
        ref = issue_refs[issue["key"]]
        marker = "created" if ref.created else "updated"
        lines.append(f"- {ref.ref} {issue['title']} ({marker})")
    return "\n".join(lines)


def dry_run(plan: dict[str, Any], errors: list[str]) -> None:
    output = {
        "valid": not errors,
        "errors": errors,
        "repo": plan.get("repo"),
        "source_spec": plan.get("source_spec"),
        "project": plan.get("project"),
        "tracker": plan.get("tracker", {}).get("title"),
        "issue_count": len(plan.get("issues", [])),
        "issues": [
            {
                "key": issue.get("key"),
                "title": issue.get("title"),
                "depends_on": issue.get("depends_on", []),
                "unblocks": issue.get("unblocks", []),
                "parallelization": issue.get("parallelization"),
                "labels": issue.get("labels", []),
            }
            for issue in plan.get("issues", [])
        ],
    }
    print(json.dumps(output, indent=2))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("plan", type=Path)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    plan = json.loads(args.plan.read_text(encoding="utf-8"))
    errors = validate_plan(plan)
    if args.dry_run:
        dry_run(plan, errors)
        return 1 if errors else 0
    if errors:
        for error in errors:
            print(f"validation error: {error}", file=sys.stderr)
        return 1

    repo = str(plan["repo"])
    source = str(plan["source_spec"])
    project = plan["project"]
    owner = str(project["owner"])

    for label in plan.get("labels", []):
        ensure_label(repo, label)

    project_number, project_url = resolve_project(owner, project)
    edit_project(owner, project_number, project)
    existing_project_urls = project_item_urls(owner, project_number)

    common_values = {"source_spec": source, "project_url": project_url}
    tracker_plan = plan["tracker"]
    tracker_ref = upsert_issue(
        repo,
        tracker_plan["title"],
        tracker_plan.get("labels", []),
        render(tracker_plan["body"], common_values, {}),
    )

    issue_refs: dict[str, IssueRef] = {}
    common_values.update({"tracker_ref": tracker_ref.ref, "tracker_url": tracker_ref.url})

    for issue in plan["issues"]:
        body = render(issue["body"], common_values, issue_refs)
        issue_refs[issue["key"]] = upsert_issue(
            repo,
            issue["title"],
            issue.get("labels", []),
            body,
        )
        status = "created" if issue_refs[issue["key"]].created else "updated"
        print(f"{status} {issue_refs[issue['key']].ref}: {issue['title']}")

    child_list = child_list_lines(issue_refs, plan["issues"])
    common_values["child_list"] = child_list

    final_tracker_body = render(tracker_plan["body"], common_values, issue_refs)
    run(["gh", "issue", "edit", str(tracker_ref.number), "--repo", repo, "--body", final_tracker_body])

    for issue in plan["issues"]:
        final_body = render(issue["body"], common_values, issue_refs)
        run(
            [
                "gh",
                "issue",
                "edit",
                str(issue_refs[issue["key"]].number),
                "--repo",
                repo,
                "--body",
                final_body,
            ]
        )

    source_url = source_spec_url(repo, source)
    if source_url:
        add_project_item_once(owner, project_number, source_url, existing_project_urls)
    add_project_item_once(owner, project_number, tracker_ref.url, existing_project_urls)
    for ref in issue_refs.values():
        add_project_item_once(owner, project_number, ref.url, existing_project_urls)

    comment = plan.get("source_comment")
    if comment and source.startswith("#"):
        body = render(comment, common_values, issue_refs)
        run(["gh", "issue", "comment", source[1:], "--repo", repo, "--body", body])

    final_urls = project_item_urls(owner, project_number)
    missing = [ref.url for ref in [tracker_ref, *issue_refs.values()] if ref.url not in final_urls]
    if missing:
        raise RuntimeError(f"project membership verification failed: {missing}")

    print(f"project: {project_url}")
    print(f"tracker: {tracker_ref.ref} {tracker_ref.url}")
    print(child_list)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
