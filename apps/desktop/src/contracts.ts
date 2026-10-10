// SPDX-License-Identifier: MPL-2.0
import type { VideoAnnotationDocument } from './video-annotation-contracts';
export type AssetKind = 'image' | 'video' | 'html' | 'svg' | 'text' | 'file';
export interface Asset { id: string; name: string; kind: AssetKind; path: string; width?: number; height?: number; originX?: number; originY?: number; scaleFactor?: number }
export type ManualDrawingKind = 'pen' | 'line' | 'arrow' | 'rect' | 'ellipse' | 'text' | 'highlighter' | 'number' | 'mosaic';
export type DrawingKind = ManualDrawingKind | 'rich';
export interface RichDrawingRef { layoutId: string; layoutSha256: string; rasterSha256: string; kind: 'table' | 'formula' | 'repair' | 'extracted'; width: number; height: number }
export interface DrawingPoint { x: number; y: number }
export interface DrawingOrigin { runId: string; userMessageId: string; toolEventId: string; groupId: string; targetHandle: string; manifestSha256: string }
export interface VisualAnnotationGrant { pluginId: string; pluginRevision: number; contributionId: string }
export interface Drawing { id: string; kind: DrawingKind; color: string; strokeWidth: number; points: DrawingPoint[]; text?: string; fontSize?: number; origin?: DrawingOrigin; rich?: RichDrawingRef }
export interface DrawingEdit { index: number; before: Drawing | null; after: Drawing | null }
export type DrawingStep = DrawingEdit | { batch: DrawingEdit[] };
export interface DrawingHistory { undo: DrawingStep[]; redo: DrawingStep[] }
export interface OcrWord { text: string; x: number; y: number; width: number; height: number }
export interface OcrDocument { engine: string; language: string; width: number; height: number; textAngle?: number | null; lines: { text: string; words: OcrWord[] }[] }
export interface OcrTarget { sceneId: string; regionId: string; backgroundId: string; drawingRevision: number; x: number; y: number; width: number; height: number }
export interface SavedTranslation { backgroundId: string; drawingRevision: number; sourceId: string; document: { version: 1; targetLanguage: string; width: number; height: number; textAngle?: number | null; lines: { id: string; source: string; text: string; box: { x: number; y: number; width: number; height: number } }[] }; overlay: Asset; selection: OcrDocument }
export interface Region { id: string; x: number; y: number; width: number; height: number; imageOverride?: Asset; drawings?: Drawing[]; drawingRevision?: number; drawingHistory?: DrawingHistory; ocr?: { backgroundId: string; drawingRevision: number; document: OcrDocument }; translation?: SavedTranslation }
export interface RegionGeometry { x: number; y: number; width: number; height: number }
export interface RegionGeometryCommand { type: 'set_region_geometry'; sceneId: string; regionId: string; backgroundId: string; sourceId: string; expectedRevision: number; from: RegionGeometry; to: RegionGeometry; editId?: string }
export interface RegionGeometryEdit { id: string; regionId: string; backgroundId: string; sourceId: string; from: RegionGeometry; to: RegionGeometry }
export interface RegionGeometryHistory { revision: number; undo: RegionGeometryEdit[]; redo: RegionGeometryEdit[] }
export interface FinishRegionGeometryEdit { type: 'finish_region_geometry_edit'; sceneId: string; editId: string }
export interface ReplayRegionGeometry { type: 'undo_region_geometry' | 'redo_region_geometry'; sceneId: string; expectedHistoryRevision: number; expectedOperationId: string; regionId: string; backgroundId: string; sourceId: string; expectedRevision: number; from: RegionGeometry }
interface DrawingTarget { sceneId: string; regionId: string; backgroundId: string; expectedRevision: number }
export type DrawingCommand = DrawingTarget & (
  | { type: 'add_drawing' | 'update_drawing'; drawing: Drawing }
  | { type: 'remove_drawing'; drawingId: string }
  | { type: 'undo_drawing' | 'redo_drawing' }
);
// Region coordinates are pixels relative to the background; item bounds are viewport fractions.
export interface VideoRange { startTicks: number; endTicks: number }
export interface VideoRangeOperation { id: string; from: VideoRange | null; to: VideoRange | null }
export interface VideoEdit { sourceId: string; sourceDurationTicks: number; revision: number; range: VideoRange | null; undo: VideoRangeOperation[]; redo: VideoRangeOperation[] }
export interface SpaceItem { id: string; asset: Asset; x: number; y: number; width: number; height: number; state?: Record<string, unknown>; videoEdit?: VideoEdit; videoAnnotations?: VideoAnnotationDocument }
export interface Reference { kind: 'region' | 'item'; id: string }
export interface Message { id: string; role: 'user' | 'assistant'; text: string; reasoning?: string; conversationStart?: number; createdAt: number; runId?: string; refs?: Reference[]; attachments?: Asset[]; toolSteps?: ToolStep[] }
export interface ToolStep { id: string; name: string; label: string; status: 'running' | 'completed' | 'failed' | 'unknown'; startedAt: number; finishedAt?: number; summary?: string }
export interface Run { id: string; status: 'running' | 'completed' | 'failed' | 'canceled'; kind?: 'chat' | 'workflow' | 'continuation'; error?: string; steps?: ToolStep[] }
export interface Scene { id: string; title: string; agentId: string; connectionId?: string | null; createdAt: number; updatedAt: number; frozen: boolean; closed: boolean; background?: Asset; blackboardLink?: { parentSceneId: string; itemId: string }; regions: Region[]; geometryHistory?: RegionGeometryHistory; items: SpaceItem[]; refs: Reference[]; draft: string; messages: Message[]; conversationStart?: number; run?: Run }
export interface MemoryProviderSelection { revision: number; bindingId: string | null }
export interface AgentProfile { id: string; name: string; instructions: string; memory: string; memoryEnabled: boolean; defaultConnectionId?: string | null; memoryProvider?: MemoryProviderSelection }
export interface MemoryEntry { id: string; agentId: string; text: string; revision: number; createdAt: number; updatedAt: number; origin?: 'legacy_memory' | 'legacy_reference' | 'manual' | 'conversation'; source?: { sceneId: string; messageId: string } }
export interface MemoryStats { agentId: string; count: number; revision: number }
export interface MemoryPage { entries: MemoryEntry[]; total: number; nextCursor?: string; revision: number }
export interface MemoryPolicy { recall: boolean; explicitRetain: boolean; completedTurnSync: boolean }
export type MemoryBindingStatus = 'provisioning' | 'ready' | 'suspended' | 'retired';
export interface MemoryBindingView {
  id: string; agentId: string; revision: number; ledgerRevision: number; providerId: string;
  pluginId: string; contributionId: string; pluginRevision: number; endpoint: string;
  enabled: boolean; status: MemoryBindingStatus; policy: MemoryPolicy; configured: boolean;
  evidenceCount: number; pendingCount: number; blockedCount: number; deletionPendingCount: number;
}
export interface AgentMemoryStatus { selection: MemoryProviderSelection; bindings: MemoryBindingView[]; nextCursor: string | null }
export type MemoryEvidenceState = 'queued' | 'accepted' | 'committed' | 'blocked' | 'unknown' | 'failed' | 'suppressed';
export type MemoryReason = 'queue_full' | 'evidence_too_large' | 'authority_changed' | 'provider_unavailable' | 'provider_rejected' | 'invalid_response' | 'outcome_unknown' | 'cancelled_before_dispatch' | 'cleanup_needs_authorization';
export interface MemoryEvidenceSource { origin: 'manual' | 'user_quote' | 'completed_turn'; sceneId: string | null; userMessageId: string | null; assistantMessageId: string | null; runId: string | null }
export interface MemoryForgetReceipt { evidenceId: string; revision: number; localSuppressed: boolean; remoteState: 'pending' | 'currently_absent' | 'confirmed_deleted' | 'unknown' | 'failed' | 'needs_authorization'; quiescence: 'proven' | 'unproven' }
export interface MemoryWriteReceipt { evidenceId: string; revision: number; operationId: string | null; state: MemoryEvidenceState; reason: MemoryReason | null }
export interface MemoryEvidenceView { id: string; bindingId: string; revision: number; source: MemoryEvidenceSource; preview: string; state: MemoryEvidenceState; reason: MemoryReason | null; forgotten: MemoryForgetReceipt | null; createdAt: number; updatedAt: number }
export interface MemoryEvidencePage { entries: MemoryEvidenceView[]; total: number; revision: number; nextCursor: string | null }
export interface MemoryEvidenceDetail { summary: MemoryEvidenceView; attributedText: string | null }
export interface MemoryProviderProbe { version: string; latencyMs: number }
export interface Connection { baseUrl: string; model: string; hasKey: boolean }
export interface ConnectionParameters { temperature?: number; top_p?: number; service_tier?: 'standard' | 'priority' }
export interface ConnectionAdvanced { protocol: 'chat_completions' | 'anthropic_messages' | 'openai_responses'; authMode: 'bearer' | 'api_key' | 'none'; requestPath?: string | null; requestParameters: ConnectionParameters }
export interface ConnectionProfile extends Connection { id: string; name: string; providerId: string; revision: number; credentialId?: string | null; advanced: ConnectionAdvanced }
export interface ProviderPreset { id: string; name: string; group: 'china' | 'global' | 'custom'; baseUrl: string; model: string; advanced: ConnectionAdvanced; searchTerms?: string; pluginId: string; pluginRevision: number; contributionId: string }
export interface ConnectionProbe { requestId: string; profile: ConnectionProfile; expectedRevision?: number; apiKey?: string }
export interface ConnectionTestResult { latencyMs: number; model: string }
export interface McpCommand { executable: string; args: string[]; cwd?: string | null }
export interface McpTool { name: string; description: string; inputSchema: Record<string, unknown> }
export interface McpGrant { agentId: string; toolNames: string[] }
export interface McpServer { id: string; label: string; revision: number; command: McpCommand; tools: McpTool[]; grants: McpGrant[]; enabled: boolean }
export interface DiscoverMcpServer { id?: string; expectedRevision?: number; label: string; command: McpCommand }
export interface Snapshot { schemaVersion: 1; revision?: number; activeSceneId: string; scenes: Scene[]; agents: AgentProfile[]; memories: MemoryEntry[]; memoryStats: MemoryStats[]; mcpServers: McpServer[]; connection: Connection; connections: ConnectionProfile[]; defaultConnectionId: string | null }
export type SceneCommand =
  | RegionGeometryCommand
  | FinishRegionGeometryEdit
  | ReplayRegionGeometry
  | { type: 'new_scene' }
  | { type: 'new_conversation'; sceneId: string }
  | { type: 'freeze_scene'; sceneId: string }
  | { type: 'close_scene'; sceneId: string }
  | { type: 'activate_scene'; sceneId: string }
  | { type: 'set_draft'; sceneId: string; draft: string }
  | { type: 'rename_scene'; sceneId: string; title: string }
  | { type: 'set_refs'; sceneId: string; refs: Reference[] }
  | { type: 'set_scene_connection'; sceneId: string; connectionId: string | null }
  | { type: 'set_default_connection'; connectionId: string | null }
  | { type: 'add_region'; sceneId: string; region: Region }
  | { type: 'update_region'; sceneId: string; region: Region }
  | { type: 'remove_region'; sceneId: string; regionId: string }
  | { type: 'update_item'; sceneId: string; item: SpaceItem }
  | { type: 'remove_item'; sceneId: string; itemId: string }
  | { type: 'save_agent'; agent: AgentProfile }
  | { type: 'save_memory'; agentId: string; id?: string; text: string; expectedRevision?: number }
  | { type: 'delete_memory'; agentId: string; id: string; expectedRevision: number }
  | { type: 'set_mcp_grants'; serverId: string; expectedRevision: number; agentId: string; toolNames: string[] }
  | { type: 'set_mcp_enabled'; serverId: string; expectedRevision: number; enabled: boolean }
  | { type: 'remove_mcp_server'; serverId: string; expectedRevision: number }
  | { type: 'set_agent'; sceneId: string; agentId: string };
export interface RunEvent { sceneId: string; runId: string; text?: string; reasoning?: string; status: 'running' | 'completed' | 'failed' | 'canceled'; error?: string }
