#!/usr/bin/env bash
# Event-specific observatory render helpers.

render_observatory_event_types() {
    cat <<'TS'
export type ObservatoryEventType =
  | "RunStarted"
  | "RunStopped"
  | "RunPaused"
  | "CycleStarted"
  | "CycleCompleted"
  | "WorkItemSelected"
  | "IssueClassified"
  | "ImplementationStarted"
  | "PRCreated"
  | "ValidationStarted"
  | "ValidationPassed"
  | "ValidationFailed"
  | "PRMerged"
  | "WorkItemBlocked"
  | "CostRecorded"
  | "OperatorIntervened"
  | "PolicyResolved"
  | "WorkerHeartbeat"
  | "ProgressUpdated";
TS
}

render_observatory_event_interface() {
    cat <<'TS'
export interface ObservatoryEvent {
  event_id: string;
  run_id: string;
  sequence: number;
  type: ObservatoryEventType;
  occurred_at: string;
  received_at?: string;
  org_id: string;
  workspace_id: string;
  project_id: string;
  repository_id: string;
  project_classification: string;
  privacy_tier: "metadata-only" | "summary" | "evidence" | "full-debug";
  operator_id: string | null;
  worker_id: string | null;
  agent_id: string | null;
  harness: string | null;
  model: string | null;
  skill_or_workflow: string | null;
  issue_url: string | null;
  pr_url: string | null;
  commit_sha: string | null;
  policy_id: string | null;
  policy_version: number | null;
  policy_digest: string | null;
  duration_ms: number | null;
  estimated_cost_usd: number | null;
  actual_cost_usd: number | null;
  risk_level: string | null;
  status: string | null;
  summary: string | null;
  progress_percent: number | null;
  progress_phase: string | null;
  current_item_title: string | null;
  current_item_url: string | null;
  queue_ready_count: number | null;
  queue_blocked_count: number | null;
  queue_claimed_count: number | null;
  queue_remaining_count: number | null;
  estimated_next_item_at: string | null;
  estimated_completion_at: string | null;
  planned_next_step: string | null;
  artifact_links: string[];
}
TS
}

render_observatory_event_constants() {
    cat <<'TS'
export interface ObservatoryEventBatch {
  events: ObservatoryEvent[];
}

export const OBSERVATORY_EVENT_REQUIRED_FIELDS = [
  "event_id",
  "run_id",
  "sequence",
  "type",
  "occurred_at",
  "org_id",
  "project_id",
  "repository_id",
  "privacy_tier",
] as const;

export const PROGRESS_UPDATED_FIELDS = [
  "progress_percent",
  "progress_phase",
  "current_item_title",
  "current_item_url",
  "queue_ready_count",
  "queue_blocked_count",
  "queue_claimed_count",
  "queue_remaining_count",
  "estimated_next_item_at",
  "estimated_completion_at",
  "planned_next_step",
] as const;
TS
}

render_observatory_event_validation() {
    cat <<'TS'
export function validateObservatoryEventBatch(batch: ObservatoryEventBatch): void {
  for (const event of batch.events) {
    validateObservatoryEvent(event);
  }
}

export function validateObservatoryEvent(event: ObservatoryEvent): void {
  for (const field of OBSERVATORY_EVENT_REQUIRED_FIELDS) {
    if (event[field] === undefined || event[field] === null || event[field] === "") {
      throw new Error(`missing required event field: ${field}`);
    }
  }
  if (!Number.isInteger(event.sequence) || event.sequence < 1) {
    throw new Error("sequence must be a positive integer");
  }
}

// Duplicate event_id is ignored.
// sequence is monotonic per run_id.
// ProgressUpdated follows the same event_id dedupe and per-run sequence ordering contract.
// Sequence gaps are exposed for UI review.
// Late events are stored by occurred_at and received_at.
TS
}

render_observatory_event_schema() {
    render_observatory_event_types
    render_observatory_event_interface
    render_observatory_event_constants
    render_observatory_event_validation
}

render_observatory_ingest_imports() {
    cat <<'TS'
import { ObservatoryEvent, ObservatoryEventBatch, validateObservatoryEventBatch } from "../../../packages/event-schema/src/events";

export interface StoredEventState {
  event_id: string;
  run_id: string;
  sequence: number;
  duplicate_ignored: boolean;
  repeated_sequence: boolean;
  sequence_gap_after: number | null;
  occurred_at: string;
  received_at: string;
}
TS
}

render_observatory_ingest_planner() {
    cat <<'TS'
export function planEventIngestion(batch: ObservatoryEventBatch, seenEventIds: Set<string>, lastSequenceByRun: Map<string, number>): StoredEventState[] {
  validateObservatoryEventBatch(batch);
  return batch.events.map((event: ObservatoryEvent) => {
    if (seenEventIds.has(event.event_id)) {
      return {
        event_id: event.event_id,
        run_id: event.run_id,
        sequence: event.sequence,
        duplicate_ignored: true,
        repeated_sequence: false,
        sequence_gap_after: null,
        occurred_at: event.occurred_at,
        received_at: event.received_at ?? "server-time",
      };
    }

    const previousSequence = lastSequenceByRun.get(event.run_id) ?? 0;
    const repeatedSequence = event.sequence <= previousSequence;
    const sequenceGapAfter = event.sequence > previousSequence + 1 ? previousSequence : null;
    seenEventIds.add(event.event_id);
    if (!repeatedSequence) {
      lastSequenceByRun.set(event.run_id, event.sequence);
    }

    return {
      event_id: event.event_id,
      run_id: event.run_id,
      sequence: event.sequence,
      duplicate_ignored: false,
      repeated_sequence: repeatedSequence,
      sequence_gap_after: sequenceGapAfter,
      occurred_at: event.occurred_at,
      received_at: event.received_at ?? "server-time",
    };
  });
}
TS
}

render_observatory_ingest_result_stub() {
    cat <<'TS'
async function ingestValidatedEvents(events: ObservatoryEvent[]): Promise<EventIngestResult> {
  return {
    accepted: events.length,
    duplicates_ignored: 0,
    repeated_sequences_flagged: 0,
    sequence_gaps_flagged: 0,
    late_events_stored: 0,
  };
}
TS
}

render_observatory_event_ingestion_contract() {
    render_observatory_ingest_imports
    render_observatory_ingest_planner
    render_observatory_ingest_result_stub
}
