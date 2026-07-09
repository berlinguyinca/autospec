#!/usr/bin/env bash
# UI-specific observatory render helpers.

render_observatory_web_app_shell() {
    cat <<'TSX'
type PollingEnvelope<T> = {
  data: T;
  server_time: string;
  poll_after_ms: number;
};

type RunProgressSnapshot = {
  run_id: string;
  status: "queued" | "running" | "blocked" | "complete" | "failed" | "stale";
  repo_full_name: string;
  progress_percent: number;
  phase: string;
  current_item: { title: string; url: string | null } | null;
  queue: { ready: number; claimed: number; blocked: number; remaining: number };
  elapsed_ms: number;
  current_item_elapsed_ms: number;
  estimated_next_item_at: string | null;
  estimated_completion_at: string | null;
  eta_confidence: "low" | "medium" | "high";
  planned_next_step: string | null;
  stale_heartbeat_warning: string | null;
  last_event_id: string | null;
  last_event_summary: string | null;
};

export const DEFAULT_POLL_AFTER_MS = 10000;

export const OPERATOR_UI_PAGES = [
  "Live Fleet",
  "Run Timeline",
  "Run Progress",
  "Work Item Detail",
  "Queue / Backlog",
  "Failures / Blockers",
  "Workers / Agents",
  "Policy Decision Inspector",
  "Cost / Duration / Outcome Reports",
] as const;

async function pollJson<T>(path: string): Promise<PollingEnvelope<T>> {
  const response = await fetch(path, { headers: { accept: "application/json" } });
  if (!response.ok) {
    throw new Error(`observatory poll failed: ${response.status}`);
  }
  const body = (await response.json()) as PollingEnvelope<T>;
  return { ...body, poll_after_ms: body.poll_after_ms ?? DEFAULT_POLL_AFTER_MS };
}

export function scheduleOperatorPoll<T>(
  path: string,
  onData: (payload: T) => void,
  onError: (error: Error) => void,
): () => void {
  let cancelled = false;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const tick = async () => {
    try {
      const envelope = await pollJson<T>(path);
      if (!cancelled) {
        onData(envelope.data);
        timer = setTimeout(tick, envelope.poll_after_ms);
      }
    } catch (error) {
      if (!cancelled) {
        onError(error instanceof Error ? error : new Error(String(error)));
        timer = setTimeout(tick, DEFAULT_POLL_AFTER_MS);
      }
    }
  };

  void tick();
  return () => {
    cancelled = true;
    if (timer) clearTimeout(timer);
  };
}

function formatMs(ms: number): string {
  const minutes = Math.floor(ms / 60000);
  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  return hours > 0 ? `${hours}h ${remainingMinutes}m` : `${remainingMinutes}m`;
}

export function ProgressBar({ progress }: { progress: RunProgressSnapshot }) {
  const percent = Math.max(0, Math.min(100, progress.progress_percent));
  const stateLabel = progress.stale_heartbeat_warning ? "stale/error state" : progress.status;

  return (
    <div className="run-progress-bar" aria-label={`${progress.repo_full_name} progress`}>
      <div className="progress-row">
        <strong>{percent}% complete</strong>
        <span>{stateLabel}</span>
      </div>
      <div role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}>
        <div style={{ width: `${percent}%` }} />
      </div>
      {progress.stale_heartbeat_warning ? (
        <p className="warning">Stale heartbeat warning: {progress.stale_heartbeat_warning}</p>
      ) : null}
    </div>
  );
}

export function LiveFleet({ runs }: { runs: RunProgressSnapshot[] }) {
  return (
    <section aria-labelledby="live-fleet-heading">
      <h1 id="live-fleet-heading">Live Fleet</h1>
      <p>10-second polling via poll_after_ms keeps active runs current without realtime streams.</p>
      <table>
        <thead>
          <tr>
            <th>Repo</th>
            <th>Status</th>
            <th>Current item</th>
            <th>Phase</th>
            <th>Queue counts</th>
            <th>ETA</th>
            <th>Progress</th>
          </tr>
        </thead>
        <tbody>
          {runs.map((run) => (
            <tr key={run.run_id}>
              <td>{run.repo_full_name}</td>
              <td>{run.status}</td>
              <td>{run.current_item?.title ?? "No active item"}</td>
              <td>{run.phase}</td>
              <td>{`${run.queue.ready} ready / ${run.queue.claimed} claimed / ${run.queue.blocked} blocked / ${run.queue.remaining} remaining`}</td>
              <td>{run.estimated_completion_at ?? "ETA unavailable"}</td>
              <td><ProgressBar progress={run} /></td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

export function RunProgress({ progress }: { progress: RunProgressSnapshot }) {
  return (
    <section aria-labelledby="run-progress-heading">
      <h2 id="run-progress-heading">Run Progress</h2>
      <ProgressBar progress={progress} />
      <dl>
        <dt>Current phase</dt>
        <dd>{progress.phase}</dd>
        <dt>Current item</dt>
        <dd>{progress.current_item?.title ?? "No active item"}</dd>
        <dt>Item elapsed time</dt>
        <dd>{formatMs(progress.current_item_elapsed_ms)}</dd>
        <dt>Queue counts</dt>
        <dd>{`${progress.queue.ready} ready, ${progress.queue.claimed} claimed, ${progress.queue.blocked} blocked, ${progress.queue.remaining} remaining`}</dd>
        <dt>ETA</dt>
        <dd>{progress.estimated_completion_at ?? "unknown"} ({progress.eta_confidence} confidence)</dd>
        <dt>Planned next step</dt>
        <dd>{progress.planned_next_step ?? "waiting for the next event"}</dd>
        <dt>Last estimate event</dt>
        <dd>{progress.last_event_summary ?? progress.last_event_id ?? "none"}</dd>
      </dl>
      {progress.stale_heartbeat_warning || progress.status === "failed" ? (
        <aside role="alert">stale/error state: {progress.stale_heartbeat_warning ?? progress.status}</aside>
      ) : null}
    </section>
  );
}

export function RunTimeline() {
  return <section><h2>Run Timeline</h2><p>Chronological activity stream from /v1/runs/:id/timeline.</p></section>;
}

export function WorkItemDetail() {
  return <section><h2>Work Item Detail</h2><p>Issue, branch, worktree, PR, commits, validations, CI, cost, blocker, and artifacts.</p></section>;
}

export function QueueBacklog() {
  return <section><h2>Queue / Backlog</h2><p>auto-implement, needs-classify, blocked issues, and priority order.</p></section>;
}

export function FailuresBlockers() {
  return <section><h2>Failures / Blockers</h2><p>Failed CI, validation errors, stale locks, quota parks, and human-needed states.</p></section>;
}

export function WorkersAgents() {
  return <section><h2>Workers / Agents</h2><p>Host, repo, PID/session, heartbeat, model, harness, lock owner, and last log line.</p></section>;
}

export function PolicyDecisionInspector() {
  return <section><h2>Policy Decision Inspector</h2><p>Resolved policy, digest, project class, risk score, privacy tier, merge permission, and rejected alternatives.</p></section>;
}
TSX
}
