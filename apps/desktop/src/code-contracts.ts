// SPDX-License-Identifier: MPL-2.0
import type { OcrTarget } from './contracts';
export interface CodeScanRequest { requestId: string; pluginId: string; revision: number; contributionId: string; target: OcrTarget; translationId: string | null }
export interface CodeValue { id: string; format: string; text: string; canOpen: boolean }
export interface CodeScanResult { requestId: string; sceneId: string; regionId: string; sourceToken: string; codes: CodeValue[] }
export interface CodeSource { key: string; request: Omit<CodeScanRequest, 'requestId'> }
export interface CodeScanView { key: string; result: CodeScanResult }
export type CodeAction = 'copy' | 'open';
