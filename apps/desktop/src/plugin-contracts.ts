// SPDX-License-Identifier: MPL-2.0
import type { ConnectionAdvanced } from './contracts';
export interface ModelConnectionTemplate { providerId: string; group: 'china' | 'global' | 'custom'; baseUrl: string; model: string; searchTerms?: string; advanced: ConnectionAdvanced }
export type PluginSource = { type: 'official' } | { type: 'local'; path: string } | { type: 'github'; repository: string; commit: string; path: string };
export type PluginContribution =
  | { id: string; title: string; kind: 'model.connection'; template: ModelConnectionTemplate }
  | { id: string; title: string; kind: 'selection.workflow'; prompt: string; accepts: ['image'] }
  | { id: string; title: string; kind: 'selection.drawing-tools'; tools: import('./contracts').ManualDrawingKind[] }
  | { id: string; title: string; kind: 'artifact.video-drawing-tools'; tools: import('./video-drawing-contracts').VideoDrawingTool[] }
  | { id: string; title: string; kind: 'selection.ocr'; engine: 'windows' }
  | { id: string; title: string; kind: 'selection.scroll' }
  | { id: string; title: string; kind: 'selection.translation' }
  | { id: string; title: string; kind: 'selection.pin' }
  | { id: string; title: string; kind: 'selection.codes'; engine: 'rxing' }
  | { id: string; title: string; kind: 'selection.recording'; engine: 'windows.wgc-mf' }
  | { id: string; title: string; kind: 'artifact.video-trim'; engine: 'windows.media-editing' }
  | { id: string; title: string; kind: 'artifact.video-gif'; engine: 'windows.media-editing-gif' }
  | { id: string; title: string; kind: 'input.speech-to-text'; engine: 'windows.sapi' }
  | { id: string; title: string; kind: 'agent.visual-annotations'; engine: 'host.vector-v1' }
  | { id: string; title: string; kind: 'recording.audio'; engine: 'windows.wasapi' }
  | { id: string; title: string; kind: 'memory.provider'; adapter: 'hindsight' };
export type PluginTag = 'agi' | 'ui-enhancement' | 'billing' | 'themes' | 'model-providers' | 'messaging' | 'memory' | 'tools' | 'system' | 'vision' | 'audio' | 'documents' | 'skills' | 'automation' | 'privacy' | 'entertainment';
export interface PluginManifest { schemaVersion: 1; id: string; version: string; name: string; description: string; publisher?: string; license: string; hostApi: 1; tags?: PluginTag[]; contributions: PluginContribution[] }
export interface PluginRecord { manifest: PluginManifest; source: PluginSource; revision: number; state: 'enabled' | 'disabled' | 'removed'; hasRollback: boolean; error?: string }
export interface PluginSnapshot { revision: number; plugins: PluginRecord[] }
export interface CatalogEntry { manifest: PluginManifest; source: PluginSource; installed?: PluginRecord }
export interface PluginCatalog { entries: CatalogEntry[]; source?: string; error?: string }
export interface PluginProposal { id: string; manifest: PluginManifest; source: PluginSource; expectedRevision?: number; sha256: string }
