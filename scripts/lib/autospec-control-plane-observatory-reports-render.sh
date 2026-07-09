#!/usr/bin/env bash
# Report-specific observatory render helpers.

render_observatory_report_catalog() {
    cat <<'TS'
export type ObservatoryReportId =
  | "project-weekly-summary"
  | "client-billing-export"
  | "open-source-maintenance-report"
  | "agent-performance-report"
  | "cost-anomaly-report"
  | "blocked-work-report"
  | "autonomous-roi-report";

export const OBSERVATORY_REPORTS = [
  { id: "project-weekly-summary", title: "Project weekly summary" },
  { id: "client-billing-export", title: "Client billing export" },
  { id: "open-source-maintenance-report", title: "Open-source maintenance report" },
  { id: "agent-performance-report", title: "Agent performance report" },
  { id: "cost-anomaly-report", title: "Cost anomaly report" },
  { id: "blocked-work-report", title: "Blocked work report" },
  { id: "autonomous-roi-report", title: "Autonomous ROI report" },
] as const;
TS
}

render_observatory_report_filters() {
    cat <<'TS'
export const OBSERVATORY_REPORT_FILTERS = [
  "date range",
  "project classification",
  "org/company",
  "workspace",
  "project",
  "repo",
  "operator",
  "worker",
  "agent/harness/model",
  "skill/workflow",
  "policy version",
  "privacy tier",
  "risk level",
  "status/outcome",
  "cost range",
  "duration range",
] as const;
TS
}

render_observatory_report_query_contract() {
    cat <<'TS'
export interface ObservatoryReportQuery {
  report_id: ObservatoryReportId;
  date_range: { from: string; to: string };
  privacy_tier: "metadata-only" | "summary" | "evidence" | "full-debug";
  project_classification?: string;
  org_company?: string;
  workspace_id?: string;
  project_id?: string;
  repo_full_name?: string;
  operator_id?: string;
  worker_id?: string;
  agent_harness_model?: string;
  skill_workflow?: string;
  policy_version?: number;
  risk_level?: string;
  status_outcome?: string;
  cost_range?: { min_usd?: number; max_usd?: number };
  duration_range?: { min_ms?: number; max_ms?: number };
}
TS
}

render_observatory_report_row_contract() {
    cat <<'TS'
export interface ObservatoryReportRow {
  org_id: string;
  workspace_id: string;
  project_id: string;
  repo_full_name: string;
  operator_id: string | null;
  worker_id: string | null;
  model: string | null;
  harness: string | null;
  skill_or_workflow: string | null;
  issue_url: string | null;
  pr_url: string | null;
  estimated_cost_usd: number | null;
  actual_cost_usd: number | null;
  duration_ms: number | null;
  blocked_time_ms: number | null;
  status_outcome: string;
  roi_summary: string | null;
}
TS
}

render_observatory_report_routes() {
    cat <<'TS'
export const OBSERVATORY_REPORT_ROUTES = [
  "GET /v1/reports/project-weekly-summary",
  "GET /v1/reports/client-billing-export",
  "GET /v1/reports/open-source-maintenance-report",
  "GET /v1/reports/agent-performance-report",
  "GET /v1/reports/cost-anomaly-report",
  "GET /v1/reports/blocked-work-report",
  "GET /v1/reports/autonomous-roi-report",
] as const;

export function scaffoldReportQuery(query: ObservatoryReportQuery): ObservatoryReportRow[] {
  void query;
  return [];
}
TS
}

render_observatory_reports_contract() {
    render_observatory_report_catalog
    render_observatory_report_filters
    render_observatory_report_query_contract
    render_observatory_report_row_contract
    render_observatory_report_routes
}

render_observatory_report_filter_ui() {
    cat <<'TSX'
import { OBSERVATORY_REPORT_FILTERS, OBSERVATORY_REPORTS } from "../../api/src/reports";

export function ReportFilters() {
  return (
    <section aria-labelledby="reports-heading">
      <h2 id="reports-heading">Cost / Duration / Outcome Reports</h2>
      <div className="report-cards">
        {OBSERVATORY_REPORTS.map((report) => <article key={report.id}>{report.title}</article>)}
      </div>
      <form aria-label="Report filters">
        {OBSERVATORY_REPORT_FILTERS.map((filter) => (
          <label key={filter}>{filter}<input name={filter.replaceAll("/", "_").replaceAll(" ", "_")} /></label>
        ))}
      </form>
    </section>
  );
}
TSX
}
