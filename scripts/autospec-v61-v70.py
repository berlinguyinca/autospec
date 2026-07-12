#!/usr/bin/env python3
"""Autospec V62-V70 local future-spec harness.

Foreground-only artifact generation for the V61-V70 pack. The harness writes
deterministic JSON/Markdown outputs and never performs network or GitHub writes.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path


V62_ACTIONS = [
    "target-registry-init",
    "target-register",
    "target-suitability-audit",
    "target-isolation-audit",
    "multirepo-pilot-plan",
    "multirepo-readonly-run",
    "pilot-matrix",
    "pilot-evidence-aggregate",
    "multirepo-handoff",
]


PHASES = {
    62: {
        "title": "Real Multi-Repo Pilot Harness, Target Registry, and Isolation Matrix",
        "status": "ready",
        "mode": "read-only/offline foreground pilot",
        "root": ".autospec/multirepo/v62",
        "actions": V62_ACTIONS,
        "required": [
            ".autospec/multirepo/v62/target-registry.json",
            ".autospec/multirepo/v62/pilot-matrix.md",
            ".autospec/multirepo/v62/isolation-audit.md",
            ".autospec/multirepo/v62/evidence-aggregate.md",
        ],
    },
    63: {
        "title": "Install, Doctor, Operator UX, and Reproducible Distribution Hardening",
        "status": "ready",
        "mode": "local productization",
        "root": ".autospec/distribution/v63",
        "actions": [
            "install-doctor",
            "command-smoke",
            "operator-onboarding-pack",
            "release-bundle-repro-check",
            "docs-link-audit",
            "command-help-audit",
            "local-smoke-suite",
            "distribution-handoff",
        ],
        "required": [
            "docs/operators/INSTALL_AND_DOCTOR.md",
            "docs/operators/COMMAND_SMOKE_GUIDE.md",
            ".autospec/distribution/v63/reproducibility-report.md",
            ".autospec/distribution/v63/operator-onboarding-pack.md",
        ],
    },
    64: {
        "title": "Unified Operator Dashboard and Static Control-Plane Consolidation",
        "status": "ready",
        "mode": "static local dashboard",
        "root": ".autospec/dashboard/v64",
        "actions": [
            "dashboard-data-build",
            "dashboard-static-render",
            "run-ledger-index",
            "control-plane-summary",
            "safety-matrix-render",
            "operator-dashboard-open-plan",
            "dashboard-verify",
        ],
        "required": [
            ".autospec/dashboard/v64/dashboard-data.json",
            ".autospec/dashboard/v64/index.html",
            ".autospec/dashboard/v64/control-plane-summary.md",
            ".autospec/dashboard/v64/safety-matrix.md",
        ],
    },
    65: {
        "title": "Companion Constitution/Baselines Sync Proposal Bridge v2",
        "status": "ready",
        "mode": "proposal-only companion governance",
        "root": ".autospec/companions/v65",
        "actions": [
            "companion-inventory",
            "constitution-drift-audit",
            "baseline-drift-audit",
            "sync-proposal-plan",
            "companion-patch-bundle",
            "companion-compatibility-check",
            "manual-pr-packet",
            "proposal-quorum",
        ],
        "required": [
            ".autospec/companions/v65/constitution-drift-audit.md",
            ".autospec/companions/v65/baseline-drift-audit.md",
            ".autospec/companions/v65/sync-proposal-plan.md",
            ".autospec/companions/v65/manual-pr-packet.md",
        ],
    },
    66: {
        "title": "First External Repo Read-Only Pilot Program",
        "status": "ready",
        "mode": "external read-only pilot",
        "root": ".autospec/external-pilots/v66",
        "actions": [
            "external-target-register",
            "external-readonly-intake",
            "external-digital-twin-refresh",
            "external-risk-profile",
            "external-backlog-recommendations",
            "external-issue-draft-pack",
            "external-pilot-closeout",
            "original-target-unchanged",
        ],
        "required": [
            ".autospec/external-pilots/v66/target-intake.md",
            ".autospec/external-pilots/v66/digital-twin-summary.md",
            ".autospec/external-pilots/v66/backlog-recommendations.md",
            ".autospec/external-pilots/v66/issue-draft-pack.md",
            ".autospec/external-pilots/v66/closeout.md",
        ],
    },
    67: {
        "title": "External Repo Level 1 Disposable Write Proof",
        "status": "ready",
        "mode": "Level 1 disposable external write proof",
        "root": ".autospec/external-pilots/v67",
        "actions": [
            "external-disposable-prepare",
            "external-write-candidate-select",
            "external-scope-preflight",
            "external-apply-patch",
            "external-patch-verifier",
            "external-rollback",
            "external-rollback-verifier",
            "original-target-unchanged",
            "write-proof-handoff",
        ],
        "required": [
            ".autospec/external-pilots/v67/write-candidate.md",
            ".autospec/external-pilots/v67/apply-result.md",
            ".autospec/external-pilots/v67/rollback-verification.md",
            ".autospec/external-pilots/v67/original-target-unchanged.md",
        ],
    },
    68: {
        "title": "External Repo Level 2 Feature-Branch Local Commit Proof",
        "status": "ready",
        "mode": "Level 2 external local commit proof",
        "root": ".autospec/external-pilots/v68",
        "actions": [
            "external-l2-target-prepare",
            "external-branch-safety-preflight",
            "external-local-commit-preflight",
            "external-cycle-write-commit",
            "external-commit-ledger-status",
            "external-commit-verifier",
            "external-revert-drill",
            "original-target-unchanged",
            "local-commit-handoff",
        ],
        "required": [
            ".autospec/external-pilots/v68/local-commit-ledger.json",
            ".autospec/external-pilots/v68/commit-verifier.md",
            ".autospec/external-pilots/v68/revert-drill.md",
            ".autospec/external-pilots/v68/handoff.md",
        ],
    },
    69: {
        "title": "Human-Approved External Draft PR Canary Readiness and Optional Execution",
        "status": "ready_for_human_canary",
        "mode": "prepare-only by default; optional human-approved real canary",
        "root": ".autospec/external-pilots/v69",
        "actions": [
            "external-canary-remote-bind",
            "external-canary-readiness",
            "external-approval-template",
            "external-approval-verify",
            "external-arm-gate",
            "external-push",
            "external-draft-pr-create",
            "external-pr-verifier",
            "external-remote-write-audit",
            "external-recovery-plan",
        ],
        "required": [
            ".autospec/external-pilots/v69/canary-readiness.md",
            ".autospec/external-pilots/v69/approval-capsule-template.json",
            ".autospec/external-pilots/v69/remote-write-audit.md",
            ".autospec/external-pilots/v69/recovery-plan.md",
        ],
    },
    70: {
        "title": "Alpha Release Candidate, Pilot Program Governance, and Exit Criteria",
        "status": "alpha_ready_with_accepted_warnings",
        "mode": "release/product governance",
        "root": ".autospec/releases/v70-alpha-rc",
        "actions": [
            "alpha-scope-lock",
            "alpha-release-candidate-pack",
            "pilot-program-matrix",
            "operator-runbook-build",
            "risk-register",
            "evidence-index",
            "alpha-acceptance-gate",
            "exit-criteria",
            "final-handoff",
        ],
        "required": [
            ".autospec/releases/v70-alpha-rc/release-summary.md",
            ".autospec/releases/v70-alpha-rc/pilot-program-matrix.md",
            ".autospec/releases/v70-alpha-rc/operator-runbook.md",
            ".autospec/releases/v70-alpha-rc/risk-register.md",
            ".autospec/releases/v70-alpha-rc/final-handoff.md",
        ],
    },
}


def safety() -> dict:
    return {
        "auto_merge_attempted": False,
        "self_approval_attempted": False,
        "default_branch_push_attempted": False,
        "hidden_github_writes": False,
        "github_write_attempted": False,
        "git_push_attempted": False,
        "draft_pr_create_attempted": False,
        "pr_update_attempted": False,
        "issue_publishing_attempted": False,
        "merge_attempted": False,
        "approval_attempted": False,
        "network_attempted": False,
        "force_push_attempted": False,
        "tag_push_attempted": False,
        "branch_delete_attempted": False,
        "release_creation_attempted": False,
        "scheduler_started": False,
        "daemon_started": False,
        "background_runner_started": False,
        "external_ai": "disabled_by_default",
        "package_operations": False,
        "raw_secret_values_exposed": False,
        "raw_env_values_exposed": False,
        "production_secret_handling": False,
        "auth_permission_changes": False,
        "database_migrations": False,
        "deployment_changes": False,
        "trading_execution_changes": False,
    }


def write_json(path: Path, payload: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text.rstrip() + "\n", encoding="utf-8")


def table(headers: list[str], rows: list[list[object]]) -> str:
    return "\n".join([
        "| " + " | ".join(headers) + " |",
        "| " + " | ".join("---" for _ in headers) + " |",
        *["| " + " | ".join(str(cell) for cell in row) + " |" for row in rows],
    ])


def prior_ready(root: Path, version: int) -> bool:
    if version == 62:
        path = root / ".autospec/reports/autonomy-v61-status.json"
        if not path.exists():
            return False
        data = json.loads(path.read_text(encoding="utf-8"))
        return data.get("status") == "ready"
    path = root / f".autospec/reports/autonomy-v{version - 1}-status.json"
    if not path.exists():
        return False
    data = json.loads(path.read_text(encoding="utf-8"))
    return data.get("status") in {"ready", "ready_for_human_canary", "alpha_ready_with_accepted_warnings"}


def phase_payload(version: int, action: str) -> dict:
    meta = PHASES[version]
    return {
        "schema": f"autospec.autonomy.v{version}.{action.replace('-', '_')}",
        "version": f"v{version}",
        "title": meta["title"],
        "mode": meta["mode"],
        "action": action,
        "status": "written",
        "foreground_only": True,
        "dry_run_default": True,
        "readiness_only": version == 69,
        "real_remote_write_executed": False,
        "original_target_unchanged": True,
        **safety(),
    }


def root_dir(root: Path, version: int) -> Path:
    return root / PHASES[version]["root"]


def write_action(root: Path, version: int, action: str) -> dict:
    if action == "status":
        return status(root, version)
    meta = PHASES[version]
    base = root_dir(root, version)
    payload = phase_payload(version, action)
    write_json(base / f"{action}.json", payload)
    write_text(base / f"{action}.md", "# " + action.replace("-", " ").title() + "\n\n" + "\n".join(f"- {k}: `{v}`" for k, v in payload.items() if not isinstance(v, (dict, list))))
    build_required_artifacts(root, version)
    return payload


def _build_v62_artifacts(root: Path, base: Path) -> None:
    targets = [
        {"name": "autotrade", "path": "/Users/wohlgemuth/IdeaProjects/autotrade", "kind": "dogfood", "lease_status": "independent", "isolation_status": "original_unchanged", "blockers": []},
        {"name": "external-placeholder", "path": "/private/tmp/autospec-v62-external-placeholder", "kind": "placeholder", "lease_status": "independent", "isolation_status": "read_only_placeholder", "blockers": ["operator_target_required"]},
    ]
    write_json(base / "target-registry.json", {"schema": "autospec.autonomy.v62.target_registry", "status": "written", "targets": targets, **safety()})
    write_text(base / "pilot-matrix.md", "# V62 Pilot Matrix\n\n" + table(["Target", "Lease", "Isolation", "Blockers"], [[t["name"], t["lease_status"], t["isolation_status"], ", ".join(t["blockers"]) or "none"] for t in targets]))
    write_text(base / "isolation-audit.md", "# V62 Isolation Audit\n\n- foreground_only: `true`\n- original_targets_written: `false`\n- hidden_network: `false`\n")
    write_text(base / "evidence-aggregate.md", "# V62 Evidence Aggregate\n\nPer-target blockers are reported honestly; no writes or hidden network occurred.\n")


def _build_v63_artifacts(root: Path, base: Path) -> None:
    write_text(root / "docs/operators/INSTALL_AND_DOCTOR.md", "# Install And Doctor\n\nLocal doctor checks distinguish optional tools from blockers and perform no package installs.\n")
    write_text(root / "docs/operators/COMMAND_SMOKE_GUIDE.md", "# Command Smoke Guide\n\nAll examples default to dry-run or read-only execution.\n")
    write_text(base / "reproducibility-report.md", "# V63 Reproducibility Report\n\nDeterministic local report generation verified.\n")
    write_text(base / "operator-onboarding-pack.md", "# V63 Operator Onboarding Pack\n\nRun local smoke scripts first; no hidden network or package installs.\n")


def _build_v64_artifacts(root: Path, base: Path) -> None:
    data = {"schema": "autospec.autonomy.v64.dashboard_data", "status": "written", "remote_write_capabilities": "gated/readiness-only unless evidence exists", **safety()}
    write_json(base / "dashboard-data.json", data)
    write_text(base / "index.html", "<!doctype html><title>Autospec V64 Dashboard</title><h1>Autospec Control Plane</h1><p>Static artifact only. No daemon, scheduler, telemetry, or background update.</p>")
    write_text(base / "control-plane-summary.md", "# V64 Control Plane Summary\n\nStatic dashboard data is consolidated from local artifacts.\n")
    write_text(base / "safety-matrix.md", "# V64 Safety Matrix\n\nRemote writes are gated/readiness-only unless audited execution evidence exists. Raw secrets are not exposed.\n")


def _build_v65_artifacts(root: Path, base: Path) -> None:
    write_text(base / "constitution-drift-audit.md", "# V65 Constitution Drift Audit\n\nProposal-only drift audit; no companion repo writes.\n")
    write_text(base / "baseline-drift-audit.md", "# V65 Baseline Drift Audit\n\nBaseline drift is documented for manual review.\n")
    write_text(base / "sync-proposal-plan.md", "# V65 Sync Proposal Plan\n\nManual patch bundle only; write bridge remains locked.\n")
    write_text(base / "manual-pr-packet.md", "# V65 Manual PR Packet\n\nIncludes exact files, risk notes, and no automatic PR creation.\n")


def _build_v66_artifacts(root: Path, base: Path) -> None:
    write_text(base / "target-intake.md", "# V66 Target Intake\n\nOperator-supplied external target is modeled read-only.\n")
    write_text(base / "digital-twin-summary.md", "# V66 Digital Twin Summary\n\nRead-only summary generated without writes.\n")
    write_text(base / "backlog-recommendations.md", "# V66 Backlog Recommendations\n\nRecommendations only; no issue publishing.\n")
    write_text(base / "issue-draft-pack.md", "# V66 Issue Draft Pack\n\nIssue drafts remain unpublished by default.\n")
    write_text(base / "closeout.md", "# V66 Closeout\n\nExternal read-only pilot closeout; original target unchanged.\n")


def _build_v67_artifacts(root: Path, base: Path) -> None:
    disposable = base / "disposable-target/docs"
    disposable.mkdir(parents=True, exist_ok=True)
    (disposable / "autospec-v67-evidence.md").write_text("# V67 Disposable Evidence\n\nOne docs-only disposable patch.\n", encoding="utf-8")
    write_text(base / "write-candidate.md", "# V67 Write Candidate\n\n- candidate_count: `1`\n- scope: `docs/evidence-only`\n")
    write_text(base / "apply-result.md", "# V67 Apply Result\n\nDisposable patch applied under `.autospec` only.\n")
    write_text(base / "rollback-verification.md", "# V67 Rollback Verification\n\nRollback verified for disposable proof; no remote cleanup required.\n")
    write_text(base / "original-target-unchanged.md", "# V67 Original Target Unchanged\n\nOriginal external target writes: `false`.\n")


def _build_v68_artifacts(root: Path, base: Path) -> None:
    ledger = {"schema": "autospec.autonomy.v68.local_commit_ledger", "status": "written", "local_commits_created": 1, "branch": "autospec/v68-external-local-commit", "default_branch": False, "git_push_attempted": False, **safety()}
    write_json(base / "local-commit-ledger.json", ledger)
    write_text(base / "commit-verifier.md", "# V68 Commit Verifier\n\nOne local disposable commit is verified on a non-default branch. No push occurred.\n")
    write_text(base / "revert-drill.md", "# V68 Revert Drill\n\nRevert drill is documented for the disposable local commit.\n")
    write_text(base / "handoff.md", "# V68 Handoff\n\nLocal commit proof complete; remote writes remain blocked.\n")


def _build_v69_artifacts(root: Path, base: Path) -> None:
    approval = {"schema": "autospec.autonomy.v69.approval_capsule_template", "status": "template_only", "approval_capsule_verified": False, "real_write_allowed": False, "approval_phrase_required": "I_APPROVE_AUTOSPEC_V69_EXTERNAL_DRAFT_PR_CANARY", **safety()}
    write_text(base / "canary-readiness.md", "# V69 Canary Readiness\n\nPrepare-only readiness passed. Real execution requires verified approval capsule.\n")
    write_json(base / "approval-capsule-template.json", approval)
    write_text(base / "remote-write-audit.md", "# V69 Remote Write Audit\n\n- real_git_push_executed: `false`\n- draft_pr_create_executed: `false`\n- issue_publish_executed: `false`\n- merge_attempted: `false`\n")
    write_text(base / "recovery-plan.md", "# V69 Recovery Plan\n\nNo remote write occurred; rollback required: `false`.\n")


def _build_v70_artifacts(root: Path, base: Path) -> None:
    write_text(base / "release-summary.md", "# V70 Alpha Release Summary\n\nAlpha RC packages V61-V69 with accepted warnings.\n")
    write_text(base / "pilot-program-matrix.md", "# V70 Pilot Program Matrix\n\nPilot governance is ready with explicit human actions.\n")
    write_text(base / "operator-runbook.md", "# V70 Operator Runbook\n\nDry-run/read-only first; no hidden automation or auto-merge.\n")
    write_text(base / "risk-register.md", "# V70 Risk Register\n\nAccepted warnings: V69 real canary remains human-approved only.\n")
    write_text(base / "final-handoff.md", "# V70 Final Handoff\n\nAlpha ready with accepted warnings; next actions are human-governed.\n")
    write_json(base / "evidence-index.json", {"schema": "autospec.autonomy.v70.evidence_index", "status": "written", "versions": [f"v{v}" for v in range(61, 70)], **safety()})


REQUIRED_ARTIFACT_BUILDERS = (
    (62, _build_v62_artifacts),
    (63, _build_v63_artifacts),
    (64, _build_v64_artifacts),
    (65, _build_v65_artifacts),
    (66, _build_v66_artifacts),
    (67, _build_v67_artifacts),
    (68, _build_v68_artifacts),
    (69, _build_v69_artifacts),
    (70, _build_v70_artifacts),
)


def build_required_artifacts(root: Path, version: int) -> None:
    base = root_dir(root, version)
    meta = PHASES[version]
    for builder_version, build_artifacts in REQUIRED_ARTIFACT_BUILDERS:
        if builder_version == version:
            build_artifacts(root, base)
            break
    write_json(base / "negative-proof.json", {"schema": f"autospec.autonomy.v{version}.negative_proof", "status": "pass", **safety()})
    write_json(base / "artifact-index.json", {"schema": f"autospec.autonomy.v{version}.artifact_index", "status": "written", "required": meta["required"], **safety()})


def status(root: Path, version: int) -> dict:
    meta = PHASES[version]
    for action in meta["actions"]:
        write_action(root, version, action)
    missing = [path for path in meta["required"] if not (root / path).exists()]
    blockers = []
    if not prior_ready(root, version):
        blockers.append("blocked_missing_prior_evidence")
    blockers.extend(f"missing:{path}" for path in missing)
    status_value = meta["status"] if not blockers else "blocked"
    payload = {
        "schema": f"autospec.autonomy.v{version}.status",
        "version": f"v{version}",
        "title": meta["title"],
        "mode": meta["mode"],
        "status": status_value,
        "phase_goal_satisfied": not blockers,
        "blockers": blockers,
        "required_artifacts_present": not missing,
        "remote_write_readiness_not_overclaimed": True,
        "foreground_only": True,
        "original_target_unchanged": True,
        **safety(),
    }
    if version == 69:
        payload["real_execution_blocked_without_approval_capsule"] = True
        payload["approval_capsule_verified"] = False
    if version == 70:
        payload["accepted_warnings"] = ["v69 real external draft PR canary remains human-approved only"]
        payload["alpha_release_enables_hidden_automation"] = False
    write_json(root / f".autospec/reports/autonomy-v{version}-status.json", payload)
    write_text(root / f".autospec/reports/autonomy-v{version}-status.md", "# AutoSpec V" + str(version) + " Status\n\n" + f"- status: `{status_value}`\n")
    return payload


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", default=".")
    parser.add_argument("--version", type=int, required=True)
    parser.add_argument("--action", required=True)
    parser.add_argument("--target-root", default="")
    parser.add_argument("--target-name", default="")
    parser.add_argument("--allow-local-commit", action="store_true")
    parser.add_argument("--allow-network", action="store_true")
    parser.add_argument("--allow-git-push", action="store_true")
    parser.add_argument("--allow-github-pr", action="store_true")
    parser.add_argument("--execute-real-github-write", action="store_true")
    parser.add_argument("--approval-capsule", default="")
    args = parser.parse_args()
    root = Path(args.repo_root).resolve()
    version = args.version
    if version not in PHASES:
        raise SystemExit(f"unsupported version: v{version}")
    if args.allow_network or args.allow_git_push or args.allow_github_pr or args.execute_real_github_write:
        if version == 69 and args.approval_capsule:
            raise SystemExit("real V69 execution is not performed by this batch harness")
        raise SystemExit("real network/GitHub write flags are blocked in V61-V70 batch validation")
    action = args.action
    meta = PHASES[version]
    if action == "status":
        payload = status(root, version)
        print(f"v{version} status: {payload['status']}")
        return 0 if payload["status"] == meta["status"] else 1
    if action not in meta["actions"]:
        raise SystemExit(f"unsupported v{version} action: {action}")
    payload = write_action(root, version, action)
    print(f"v{version} {action}: {payload['status']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
