#!/usr/bin/env python3
"""Recipe lookup and the rule-to-recipe plan.

Extracted from autospec-autonomy-v2-lib.py to bring that file under the repo's
file-size gate. rule_to_recipe was 76 LOC and is split into recipe matching,
status resolution, plan-row construction, and markdown rendering. The recipe list
is now read once per run instead of once per rule, which is the same data with
fewer reads; the generated artifacts were verified byte-identical.
"""

from __future__ import annotations

from pathlib import Path

from autospec_autonomy_capabilities import load_capability_statuses
from autospec_autonomy_io import load_json, reports, state, write_json, write_text
from autospec_autonomy_recipes import recipe_index

ACTIONABLE_RULE_STATUSES = {"fail", "partial", "unknown", "manual_review"}
ACTIVE_CAPABILITY_STATUSES = {"enabled", "experimental"}


def recipes(root: Path) -> list[dict]:
    data = load_json(state(root) / "implementation-recipes.json", {})
    if not data.get("recipes"):
        recipe_index(root)
        data = load_json(state(root) / "implementation-recipes.json", {})
    return data.get("recipes", [])


def rule_results(root: Path) -> list[dict]:
    data = load_json(state(root) / "rule-check-results.json", load_json(reports(root) / "rule-check-results.json", {"results": []}))
    return data.get("results", [])


def find_recipe(root: Path, rid: str) -> dict | None:
    return next((entry for entry in recipes(root) if entry["id"] == rid), None)


def _matches_rule(entry: dict, rule: dict) -> bool:
    hay = " ".join(entry.get("applies_to_rules", []))
    return rule.get("rule_id", "") in hay or rule.get("check_type", "") in hay


def _match_recipe(entries: list[dict], rule: dict) -> dict | None:
    """First recipe whose applies_to_rules mentions this rule id or check type."""
    return next((entry for entry in entries if _matches_rule(entry, rule)), None)


def _plan_status(best: dict | None, statuses: dict[str, str]) -> str:
    if not best:
        return "unsupported"
    if statuses.get(best["capability"]) not in ACTIVE_CAPABILITY_STATUSES:
        return "recipe_available_but_disabled"
    if best["risk"]["requires_human_guidance"]:
        return "requires_human_guidance"
    if best["implementation"]["mode"] == "planning_only":
        return "planning_only"
    return "recipe_available"


UNMATCHED_RECIPE_FIELDS = {
    "best_recipe": "",
    "required_capability": "",
    "estimated_risk": "unknown",
    "expected_files": {},
    "required_tests": [],
    "required_validation": [],
    "architecture_review_required": False,
}


def _recipe_fields(best: dict | None) -> dict:
    """Recipe-derived plan fields, or the unmatched defaults."""
    if not best:
        return dict(UNMATCHED_RECIPE_FIELDS)
    return {
        "best_recipe": best["id"],
        "required_capability": best["capability"],
        "estimated_risk": best["risk"]["level"],
        "expected_files": best["implementation"]["expected_files"],
        "required_tests": best["test_plan"]["generated_tests"],
        "required_validation": best["validation"]["commands"],
        "architecture_review_required": bool(best["risk"]["requires_architecture_review"]),
    }


def _plan_row(rule: dict, best: dict | None, status: str) -> dict:
    return {
        "rule_id": rule.get("rule_id"),
        "status": status,
        "fallback_recipe": "planning_only",
        "worker_eligibility": status == "recipe_available",
        "human_guidance_required": status == "requires_human_guidance",
        "target_app_runtime_required": False,
        **_recipe_fields(best),
    }


def _section(plans: list[dict], predicate) -> str:
    return "\n".join(f"- `{p['rule_id']}`" for p in plans if predicate(p)) or "- None."


def _plan_markdown(plans: list[dict]) -> str:
    return "\n".join([
        "# Rule-to-Recipe Plan",
        "",
        "## Summary",
        "",
        f"- Plans: {len(plans)}",
        "",
        "## Implementable now",
        "",
        "\n".join(f"- `{p['rule_id']}` -> `{p['best_recipe']}`" for p in plans if p["status"] == "recipe_available") or "- None.",
        "",
        "## Scaffoldable now",
        "",
        _section(plans, lambda p: p["status"] in {"recipe_available", "planning_only"}),
        "",
        "## Planning-only",
        "",
        _section(plans, lambda p: p["status"] == "planning_only"),
        "",
        "## Requires human guidance",
        "",
        _section(plans, lambda p: p["human_guidance_required"]),
        "",
        "## Unsupported",
        "",
        _section(plans, lambda p: p["status"] == "unsupported"),
        "",
        "## Recommended next issues",
        "",
        "- Build patch plans for recipe_available rows.",
    ])


def rule_to_recipe(root: Path, rule_filter: str = "") -> int:
    statuses = load_capability_statuses(root)
    entries = recipes(root)
    plans = []
    for rule in rule_results(root):
        if rule_filter and rule.get("rule_id") != rule_filter:
            continue
        if rule.get("status") not in ACTIONABLE_RULE_STATUSES:
            continue
        best = _match_recipe(entries, rule)
        plans.append(_plan_row(rule, best, _plan_status(best, statuses)))
    payload = {"schema": 1, "plans": plans}
    write_json(reports(root) / "rule-to-recipe-plan.json", payload)
    write_json(state(root) / "rule-to-recipe-plan.json", payload)
    write_text(reports(root) / "rule-to-recipe-plan.md", _plan_markdown(plans))
    return 0
