// SPDX-License-Identifier: MPL-2.0
export interface CapturePreferenceValues {
  captureDelaySeconds: 0 | 3 | 5;
  includeCursor: boolean;
  defaultImageFormat: 'png' | 'jpeg';
}
export interface CapturePreferences extends CapturePreferenceValues { version: 1; revision: number }
export interface SystemPreferenceValues {
  networkProxyMode: 'system' | 'direct' | 'custom';
  networkProxyUrl: string;
  launchAtStartup: boolean;
  allowScreenShare: boolean;
}
export interface SystemPreferences extends SystemPreferenceValues { version: 1; revision: number; startupRegistered: boolean; startupWarning?: string | null }
export function validSystemPreferences(value: unknown): value is SystemPreferences {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const v = value as SystemPreferences;
  return v.version === 1 && Number.isSafeInteger(v.revision) && v.revision >= 0
    && ['system', 'direct', 'custom'].includes(v.networkProxyMode)
    && typeof v.networkProxyUrl === 'string' && v.networkProxyUrl.length <= 2048 && !/[\0\r\n]/.test(v.networkProxyUrl)
    && typeof v.launchAtStartup === 'boolean' && typeof v.allowScreenShare === 'boolean' && typeof v.startupRegistered === 'boolean'
    && (v.startupWarning === undefined || v.startupWarning === null || typeof v.startupWarning === 'string' && v.startupWarning.length <= 4096);
}
export const sameSystemValues = (a: SystemPreferenceValues, b: SystemPreferenceValues) => a.networkProxyMode === b.networkProxyMode && a.networkProxyUrl === b.networkProxyUrl && a.launchAtStartup === b.launchAtStartup && a.allowScreenShare === b.allowScreenShare;
export interface SettingsInfo {
  application: string;
  version: string;
  commit?: string | null;
  channel: 'development' | 'alpha' | 'stable';
  platform: 'windows';
  updateSupport: 'unconfigured' | 'manual_only' | 'signed_installer';
  dataDirectory: string;
  license: string;
  noticesAvailable: boolean;
}
export type LicenseDocumentKind = 'license' | 'notices';
export interface LicenseDocument { title: string; text: string }
export interface DataDirectoryState {
  path: string;
  generation: number;
  phase: 'idle' | 'requested' | 'copying' | 'verified' | 'activated' | 'committed' | 'needs_recovery' | 'failed' | 'rolled_back';
  canChange: boolean;
}
export interface DataDirectoryProposal { proposalId: string; path: string; generation: number }
const dataPath = (value: unknown): value is string => typeof value === 'string' && value.length > 0 && value.length <= 32768 && !value.includes('\0');
export function validDataDirectoryState(value: unknown): value is DataDirectoryState {
  if (!value || typeof value !== 'object') return false;
  const v = value as DataDirectoryState;
  return dataPath(v.path) && Number.isSafeInteger(v.generation) && v.generation >= 0 && typeof v.canChange === 'boolean' && ['idle', 'requested', 'copying', 'verified', 'activated', 'committed', 'needs_recovery', 'failed', 'rolled_back'].includes(v.phase);
}
export function validDataDirectoryProposal(value: unknown): value is DataDirectoryProposal {
  if (!value || typeof value !== 'object') return false;
  const v = value as DataDirectoryProposal;
  return dataPath(v.path) && Number.isSafeInteger(v.generation) && v.generation >= 0 && typeof v.proposalId === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(v.proposalId);
}
export const sameCaptureValues = (a: CapturePreferenceValues, b: CapturePreferenceValues) => a.captureDelaySeconds === b.captureDelaySeconds && a.includeCursor === b.includeCursor && a.defaultImageFormat === b.defaultImageFormat;
export function validCapturePreferences(value: unknown): value is CapturePreferences {
  if (!value || typeof value !== 'object') return false;
  const v = value as CapturePreferences;
  return v.version === 1 && Number.isSafeInteger(v.revision) && v.revision >= 0 && [0, 3, 5].includes(v.captureDelaySeconds) && typeof v.includeCursor === 'boolean' && ['png', 'jpeg'].includes(v.defaultImageFormat);
}
export function validSettingsInfo(value: unknown): value is SettingsInfo {
  if (!value || typeof value !== 'object') return false;
  const v = value as SettingsInfo;
  return ['application', 'version', 'dataDirectory', 'license'].every(key => typeof v[key as keyof SettingsInfo] === 'string' && String(v[key as keyof SettingsInfo]).length > 0 && String(v[key as keyof SettingsInfo]).length <= 32768) && (v.commit === undefined || v.commit === null || typeof v.commit === 'string' && v.commit.length <= 128) && ['development', 'alpha', 'stable'].includes(v.channel) && v.platform === 'windows' && ['unconfigured', 'manual_only', 'signed_installer'].includes(v.updateSupport) && typeof v.noticesAvailable === 'boolean';
}
export function validLicenseDocument(value: unknown): value is LicenseDocument {
  if (!value || typeof value !== 'object') return false;
  const v = value as LicenseDocument;
  return typeof v.title === 'string' && v.title.length > 0 && v.title.length <= 160 && typeof v.text === 'string' && new TextEncoder().encode(v.text).length <= 8 * 1024 * 1024;
}
export function updateSupportLabel(value: SettingsInfo['updateSupport']): string {
  return { unconfigured: '尚未配置在线更新', manual_only: '手动更新', signed_installer: '已配置签名更新' }[value];
}
