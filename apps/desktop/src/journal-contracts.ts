// SPDX-License-Identifier: MPL-2.0
export type RecordedRunStatus = 'running' | 'completed' | 'failed' | 'canceled' | 'interrupted';
export type ContinueBlockedReason = 'running' | 'scene_closed' | 'scene_frozen' | 'scene_inactive' | 'no_confirmed_content' | 'legacy_not_recorded' | 'connection_unselected' | 'mandatory_context_too_large' | 'selection_too_large' | 'context_unavailable' | 'conversation_changed';
export type ContentUnavailable = 'oversized' | 'unsupported_content' | 'memory_policy' | 'history_policy' | 'not_received' | 'invalid_response' | 'legacy_not_recorded';
export interface RunJournalSummary {
  runId: string; sceneId: string; agentId: string; userMessageId: string;
  kind: 'chat' | 'workflow' | 'continuation'; status: RecordedRunStatus;
  revision: number; checkpointSeq: number; createdAt: number; updatedAt: number;
  canContinue: boolean; continueBlockedReason: ContinueBlockedReason | null;
}
export interface JournalEntrySummary {
  id: string; sequence: number; kind: 'model' | 'tool'; phase: 'prepared' | 'dispatched' | 'responded' | 'not_sent' | 'unknown';
  round: number; toolCallId: string | null; label: string; startedAt: number; finishedAt: number | null;
  returnedError: boolean | null; contentBytes: number; contentSha256: string | null;
  contentUnavailable: ContentUnavailable | null; selectable: boolean;
  responseKind: 'complete' | 'rpc_error' | 'input_required' | 'task' | 'local_commit' | 'local_read' | null;
  observedEncoding: string | null; observedBytes: number | null; observedSha256: string | null;
}
export interface JournalPage { summary: RunJournalSummary; entries: JournalEntrySummary[]; continuation: ContinuationDecision; nextCursor: string | null }
export type JournalContent = { type: 'text'; text: string } | { type: 'json'; value: unknown } | { type: 'omitted'; reason: ContentUnavailable };
export type ToolBinding =
  | { type: 'mcp'; serverId: string; revision: number; name: string; advertisedAlias: string }
  | { type: 'local'; name: string }
  | { type: 'external_memory'; bindingId: string; bindingRevision: number; pluginId: string; pluginRevision: number; contributionId: string; name: string }
  | { type: 'rejected'; advertisedName: string };
export type EvidenceReference =
  | { type: 'local_memory'; id: string; revision: number }
  | { type: 'external_memory'; bindingId: string; evidenceId: string; operationId: string | null; revision: number }
  | { type: 'history'; sceneId: string; messageId: string };
export interface JournalEntryDetail {
  runId: string; journalRevision: number; entry: JournalEntrySummary; binding: ToolBinding | null;
  argumentsWireSha256: string | null; content: JournalContent; references: EvidenceReference[];
}
// Rendering adapter; native content is converted to literal text, never markup.
export interface JournalEntryView {
  runId: string; journalRevision: number; entry: JournalEntrySummary;
  literal: string; unavailable: ContentUnavailable | null;
}
export interface ContinueFromRecord {
  sceneId: string; sourceRunId: string; expectedJournalRevision: number; expectedCheckpointSeq: number;
  expectedConnectionId: string; expectedConnectionRevision: number; selectedEventIds: string[] | null;
}
export interface JournalIdentity { sceneId: string; runId: string }
export interface ContinuationDecision {
  sourceRunId: string;
  journalRevision: number; checkpointSeq: number; budgetBytes: number; projectionBytes: number; defaultProjectionBytes: number;
  defaultEventIds: string[]; selectedEventIds: string[]; selectionRequired: boolean;
  ready: boolean; blockedReason: ContinueBlockedReason | null;
}
export interface JournalChanged { sceneId: string; runId: string; revision: number }
export interface PreviewRunContinuation {
  sceneId: string; sourceRunId: string; expectedJournalRevision: number;
  expectedCheckpointSeq: number; selectedEventIds: string[] | null;
}
export interface JournalReadPort {
  page(input: JournalIdentity & { cursor?: string; limit: number }): Promise<JournalPage | null>;
  detail(input: JournalIdentity & { eventId: string; expectedJournalRevision: number }): Promise<JournalEntryView>;
}
