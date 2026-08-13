#!/usr/bin/env python3
"""Validate autospec's security/database generation artifact."""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any


SCHEMA = "autospec.security_database.v1"
REQUIRED_LISTS = (
    "required_sections",
    "evidence",
    "facts",
    "assumptions",
    "priority_order",
    "threats",
    "blocking_prerequisites",
    "controls",
    "negative_tests",
    "residual_risks",
    "atomic_groups",
    "issues",
)
EVIDENCE_STATUSES = {"verified", "assumed", "blocking", "accepted"}
PREREQUISITE_STATUSES = {"verified", "blocking"}
CATASTROPHIC = {"data_loss", "unauthorized_disclosure"}
AUTHORITATIVE_OWNERS = {"database", "platform"}


class Findings:
    def __init__(self) -> None:
        self.items: list[dict[str, str]] = []

    def add(self, rule_id: str, message: str) -> None:
        item = {"rule_id": rule_id, "message": message}
        if item not in self.items:
            self.items.append(item)


def usage() -> str:
    return "usage: validate-security-artifact.py [--json] <artifact.yml>"


def emit(findings: Findings, json_mode: bool) -> int:
    if json_mode:
        print(json.dumps(findings.items, sort_keys=True))
    else:
        for item in findings.items:
            print(f"{item['rule_id']}: {item['message']}", file=sys.stderr)
    return min(len(findings.items), 64)


def mapping_rows(data: dict[str, Any], name: str, findings: Findings) -> list[dict[str, Any]]:
    value = data.get(name)
    if not isinstance(value, list):
        findings.add("PROFILE_SCHEMA_INVALID", f"{name} must be a list")
        return []
    rows: list[dict[str, Any]] = []
    for index, row in enumerate(value):
        if not isinstance(row, dict):
            findings.add("PROFILE_SCHEMA_INVALID", f"{name}[{index}] must be a mapping")
        else:
            rows.append(row)
    return rows


def string_list(row: dict[str, Any], field: str, context: str, findings: Findings) -> list[str]:
    value = row.get(field)
    if not isinstance(value, list) or any(not isinstance(item, str) or not item for item in value):
        findings.add("PROFILE_SCHEMA_INVALID", f"{context}.{field} must be a list of non-empty strings")
        return []
    return value


def indexed(rows: list[dict[str, Any]], field: str, context: str, findings: Findings) -> dict[str, dict[str, Any]]:
    result: dict[str, dict[str, Any]] = {}
    for index, row in enumerate(rows):
        value = row.get(field)
        if not isinstance(value, str) or not value:
            findings.add("PROFILE_SCHEMA_INVALID", f"{context}[{index}].{field} must be a non-empty string")
        elif value in result:
            findings.add("PROFILE_SCHEMA_INVALID", f"duplicate {context} {field} {value}")
        else:
            result[value] = row
    return result


def validate(data: Any) -> Findings:
    findings = Findings()
    if not isinstance(data, dict):
        findings.add("PROFILE_SCHEMA_INVALID", "artifact root must be a mapping")
        return findings

    if data.get("schema") != SCHEMA:
        findings.add("PROFILE_SCHEMA_INVALID", f"schema must equal {SCHEMA}")
    if data.get("feature_profile") != "security_database":
        findings.add("PROFILE_SCHEMA_INVALID", "feature_profile must equal security_database")
    if not isinstance(data.get("spec_path"), str) or not data.get("spec_path"):
        findings.add("PROFILE_SCHEMA_INVALID", "spec_path must be a non-empty string")
    for name in REQUIRED_LISTS:
        if not isinstance(data.get(name), list):
            findings.add("PROFILE_SCHEMA_INVALID", f"{name} must be a list")

    sections = [value for value in data.get("required_sections", []) if isinstance(value, str)]
    if not data.get("priority_order") or any(
        not isinstance(value, str) or not value for value in data.get("priority_order", [])
    ):
        findings.add("PROFILE_SCHEMA_INVALID", "priority_order must contain non-empty strings")
    evidence_rows = mapping_rows(data, "evidence", findings)
    prerequisite_rows = mapping_rows(data, "blocking_prerequisites", findings)
    control_rows = mapping_rows(data, "controls", findings)
    threat_rows = mapping_rows(data, "threats", findings)
    test_rows = mapping_rows(data, "negative_tests", findings)
    atomic_rows = mapping_rows(data, "atomic_groups", findings)
    issue_rows = mapping_rows(data, "issues", findings)

    evidence = indexed(evidence_rows, "id", "evidence", findings)
    prerequisites = indexed(prerequisite_rows, "id", "blocking_prerequisites", findings)
    controls = indexed(control_rows, "id", "controls", findings)
    threats = indexed(threat_rows, "id", "threats", findings)
    negative_tests = indexed(test_rows, "id", "negative_tests", findings)
    atomic_groups = indexed(atomic_rows, "id", "atomic_groups", findings)
    issues = indexed(issue_rows, "key", "issues", findings)

    for evidence_id, row in evidence.items():
        if row.get("status") not in EVIDENCE_STATUSES:
            findings.add("PROFILE_SCHEMA_INVALID", f"evidence {evidence_id} has an invalid status")
        if not isinstance(row.get("source"), str) or not row.get("source"):
            findings.add("PROFILE_SCHEMA_INVALID", f"evidence {evidence_id} requires source")

    for prerequisite_id, row in prerequisites.items():
        if row.get("status") not in PREREQUISITE_STATUSES:
            findings.add("PROFILE_SCHEMA_INVALID", f"prerequisite {prerequisite_id} has an invalid status")
        evidence_id = row.get("evidence")
        if evidence_id not in evidence:
            findings.add("PROFILE_SCHEMA_INVALID", f"prerequisite {prerequisite_id} references unknown evidence {evidence_id}")

    for control_id, row in controls.items():
        verification = string_list(row, "verification", f"control {control_id}", findings)
        if not verification:
            findings.add("CONTROL_WITHOUT_TEST", f"control {control_id} has no negative test")
        for test_id in verification:
            if test_id not in negative_tests:
                findings.add("PROFILE_SCHEMA_INVALID", f"control {control_id} references unknown negative test {test_id}")
        if row.get("failure_consequence") in CATASTROPHIC and (
            row.get("owner") not in AUTHORITATIVE_OWNERS or row.get("authority") != "authoritative"
        ):
            findings.add(
                "AUTHORITATIVE_CONTROL_MISSING",
                f"control {control_id} requires an authoritative database or platform owner",
            )

    controlled_threats = {row.get("threat_id") for row in controls.values()}
    for threat_id in threats:
        if threat_id not in controlled_threats:
            findings.add("THREAT_WITHOUT_CONTROL", f"threat {threat_id} has no control")
    for control_id, row in controls.items():
        threat_id = row.get("threat_id")
        if threat_id not in threats:
            findings.add("PROFILE_SCHEMA_INVALID", f"control {control_id} references unknown threat {threat_id}")

    for index, risk in enumerate(mapping_rows(data, "residual_risks", findings)):
        if risk.get("status") != "accepted" or not isinstance(risk.get("summary"), str) or not risk.get("summary"):
            findings.add("PROFILE_SCHEMA_INVALID", f"residual_risks[{index}] must be accepted with a summary")

    issue_sections: set[str] = set()
    owned_tests: set[str] = set()
    owned_controls: set[str] = set()
    atomic_owners: dict[str, list[str]] = {}
    produced_by: dict[str, list[str]] = {}
    graph: dict[str, list[str]] = {}
    for issue_key, row in issues.items():
        consumed_evidence = string_list(row, "evidence", f"issue {issue_key}", findings)
        produces = string_list(row, "produces", f"issue {issue_key}", findings)
        consumes = string_list(row, "consumes", f"issue {issue_key}", findings)
        consumed_prerequisites = string_list(row, "prerequisites", f"issue {issue_key}", findings)
        consumed_controls = string_list(row, "controls", f"issue {issue_key}", findings)
        issue_tests = string_list(row, "negative_tests", f"issue {issue_key}", findings)
        covers = string_list(row, "covers", f"issue {issue_key}", findings)
        dependencies = string_list(row, "depends_on", f"issue {issue_key}", findings)
        labels = string_list(row, "labels", f"issue {issue_key}", findings)
        groups = string_list(row, "atomic_groups", f"issue {issue_key}", findings)
        graph[issue_key] = dependencies
        if not produces:
            findings.add("PROFILE_SCHEMA_INVALID", f"issue {issue_key} must declare at least one produced contract")
        if not consumes:
            findings.add("PROFILE_SCHEMA_INVALID", f"issue {issue_key} must declare at least one consumed input")
        issue_sections.update(covers)
        owned_tests.update(issue_tests)
        owned_controls.update(consumed_controls)
        for contract in produces:
            produced_by.setdefault(contract, []).append(issue_key)

        for evidence_id in consumed_evidence:
            if evidence_id not in evidence:
                findings.add("PROFILE_SCHEMA_INVALID", f"issue {issue_key} references unknown evidence {evidence_id}")
            elif evidence[evidence_id].get("status") != "verified":
                findings.add("EVIDENCE_UNRESOLVED", f"issue {issue_key} consumes non-verified evidence {evidence_id}")
        for prerequisite_id in consumed_prerequisites:
            if prerequisite_id not in prerequisites:
                findings.add("PROFILE_SCHEMA_INVALID", f"issue {issue_key} references unknown prerequisite {prerequisite_id}")
                continue
            prerequisite = prerequisites[prerequisite_id]
            if issue_key not in prerequisite.get("gates", []):
                findings.add(
                    "PROFILE_SCHEMA_INVALID",
                    f"issue {issue_key} references prerequisite {prerequisite_id} without a gate mapping",
                )
            if prerequisite.get("status") != "verified" and (
                "autospec:blocked-prerequisite" not in labels or "auto-implement" in labels
            ):
                findings.add(
                    "BLOCKING_PREREQUISITE_QUEUED",
                    f"issue {issue_key} must be blocked while prerequisite {prerequisite_id} is blocking",
                )
        for control_id in consumed_controls:
            if control_id not in controls:
                findings.add("PROFILE_SCHEMA_INVALID", f"issue {issue_key} references unknown control {control_id}")
        for test_id in issue_tests:
            if test_id not in negative_tests:
                findings.add("PROFILE_SCHEMA_INVALID", f"issue {issue_key} references unknown negative test {test_id}")
        for dependency in dependencies:
            if dependency not in issues:
                findings.add("DEPENDENCY_UNKNOWN", f"issue {issue_key} depends on unknown issue {dependency}")
        for group_id in groups:
            if group_id not in atomic_groups:
                findings.add("PROFILE_SCHEMA_INVALID", f"issue {issue_key} references unknown atomic group {group_id}")
            atomic_owners.setdefault(group_id, []).append(issue_key)

    for section in sections:
        if section not in issue_sections:
            findings.add("SPEC_SECTION_UNCOVERED", f"required spec section {section} has no issue owner")
    for test_id in negative_tests:
        if test_id not in owned_tests:
            findings.add("NEGATIVE_TEST_UNOWNED", f"negative test {test_id} has no issue owner")
    for control_id in controls:
        if control_id not in owned_controls:
            findings.add("CONTROL_UNOWNED", f"control {control_id} has no issue owner")

    for prerequisite_id, row in prerequisites.items():
        gates = string_list(row, "gates", f"prerequisite {prerequisite_id}", findings)
        if not gates:
            findings.add("PROFILE_SCHEMA_INVALID", f"prerequisite {prerequisite_id} must gate at least one issue")
        for issue_key in gates:
            issue = issues.get(issue_key)
            if issue is None:
                findings.add(
                    "PROFILE_SCHEMA_INVALID",
                    f"prerequisite {prerequisite_id} gates unknown issue {issue_key}",
                )
                continue
            if prerequisite_id not in issue.get("prerequisites", []):
                findings.add(
                    "PROFILE_SCHEMA_INVALID",
                    f"prerequisite {prerequisite_id} gates issue {issue_key} without a reverse reference",
                )
            labels = issue.get("labels", [])
            if row.get("status") == "blocking" and (
                "autospec:blocked-prerequisite" not in labels or "auto-implement" in labels
            ):
                findings.add(
                    "BLOCKING_PREREQUISITE_QUEUED",
                    f"issue {issue_key} must be blocked while prerequisite {prerequisite_id} is blocking",
                )

    for group_id, row in atomic_groups.items():
        member_owners: set[str] = set()
        for member in string_list(row, "members", f"atomic group {group_id}", findings):
            owners = produced_by.get(member, [])
            if not owners:
                findings.add("ATOMIC_CONTRACT_UNOWNED", f"atomic group {group_id} member {member} has no issue owner")
            member_owners.update(owners)
        declared_owners = set(atomic_owners.get(group_id, []))
        if len(member_owners) > 1 or len(declared_owners) > 1 or member_owners != declared_owners:
            owners = sorted(member_owners | declared_owners)
            findings.add("ATOMIC_CONTRACT_SPLIT", f"atomic group {group_id} is split across {', '.join(owners)}")

    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(issue_key: str, path: list[str]) -> None:
        if issue_key in visiting:
            cycle_start = path.index(issue_key)
            cycle = path[cycle_start:] + [issue_key]
            findings.add("DEPENDENCY_CYCLE", "issue dependency cycle: " + " -> ".join(cycle))
            return
        if issue_key in visited:
            return
        visiting.add(issue_key)
        for dependency in graph.get(issue_key, []):
            if dependency in issues:
                visit(dependency, path + [dependency])
        visiting.remove(issue_key)
        visited.add(issue_key)

    for issue_key in issues:
        visit(issue_key, [issue_key])

    return findings


def main(argv: list[str]) -> int:
    json_mode = False
    args = argv[1:]
    if "--help" in args or "-h" in args:
        print(usage())
        return 0
    if "--json" in args:
        json_mode = True
        args = [arg for arg in args if arg != "--json"]
    if len(args) != 1:
        findings = Findings()
        findings.add("PROFILE_SCHEMA_INVALID", usage())
        return emit(findings, json_mode)

    try:
        import yaml
    except Exception as exc:
        findings = Findings()
        findings.add("PROFILE_SCHEMA_INVALID", f"PyYAML is unavailable: {exc}")
        return emit(findings, json_mode)

    try:
        with Path(args[0]).open(encoding="utf-8") as handle:
            data = yaml.safe_load(handle)
    except (OSError, yaml.YAMLError) as exc:
        findings = Findings()
        findings.add("PROFILE_SCHEMA_INVALID", f"cannot load artifact: {exc}")
        return emit(findings, json_mode)

    return emit(validate(data), json_mode)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
