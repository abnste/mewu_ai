// SPDX-License-Identifier: MPL-2.0
export type VoiceLanguage = 'system' | 'zh-CN' | 'en-US';
export interface VoiceCapabilities { supported: boolean; languages: Array<{ tag: string; name: string }>; unavailableReason?: string }
export interface VoiceRequest { requestId: string; pluginId: string; revision: number; contributionId: string; sceneId: string; language: VoiceLanguage }
export interface VoiceResult { requestId: string; sceneId: string; text: string; language: string; confidence: 'recognized' | 'candidate' }
export interface VoiceProgress { requestId: string; sceneId: string; phase: 'starting' | 'listening' | 'stopping' }
export interface VoicePending extends VoiceProgress { awaitingComposition: boolean }
