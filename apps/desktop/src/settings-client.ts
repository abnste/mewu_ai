// SPDX-License-Identifier: MPL-2.0
import { validCapturePreferences, validDataDirectoryProposal, validDataDirectoryState, validLicenseDocument, validSettingsInfo, type CapturePreferences, type CapturePreferenceValues, type DataDirectoryProposal, type LicenseDocumentKind } from './settings-contracts';
import { validSystemPreferences, type SystemPreferences, type SystemPreferenceValues } from './settings-contracts';
import { validDataCategory, validDataUsage, validDataCleanupReceipt, type DataCategory } from './settings-contracts';
export interface SettingsTransport {
  invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
  listen: (event: string, receive: (payload: unknown) => void) => Promise<() => void>;
}
export function settingsClient(transport: SettingsTransport) {
  async function getSystemPreferences() {
    const value = await transport.invoke('get_system_preferences');
    if (!validSystemPreferences(value)) throw new Error('系统设置状态无效');
    return value;
  }
  async function getCapturePreferences() {
    const value = await transport.invoke('get_capture_preferences');
    if (!validCapturePreferences(value)) throw new Error('截图设置状态无效');
    return value;
  }
  return {
    getSystemPreferences,
    async saveSystemPreferences(expectedRevision: number, value: SystemPreferenceValues) {
      const receipt = await transport.invoke('set_system_preferences', { expectedRevision, ...value });
      if (!validSystemPreferences(receipt)) throw new Error('系统设置状态无效');
      return receipt;
    },
    async watchSystemPreferences(accept: (state: SystemPreferences) => void) {
      let stopped = false, revision = -1, eventGeneration = 0;
      const receive = (value: unknown) => { if (!stopped && validSystemPreferences(value) && value.revision >= revision) { revision = value.revision; accept(value); } };
      const stop = await transport.listen('system-preferences-changed', value => { if (validSystemPreferences(value)) eventGeneration++; receive(value); });
      const beforeRead = eventGeneration;
      try { const value = await getSystemPreferences(); if (eventGeneration === beforeRead) receive(value); }
      catch (error) { stopped = true; stop(); throw error; }
      return () => { stopped = true; stop(); };
    },
    getCapturePreferences,
    async saveCapturePreferences(expectedRevision: number, value: CapturePreferenceValues) {
      const receipt = await transport.invoke('set_capture_preferences', { expectedRevision, ...value });
      if (!validCapturePreferences(receipt)) throw new Error('截图设置状态无效');
      return receipt;
    },
    async watchCapturePreferences(accept: (state: CapturePreferences) => void) {
      let stopped = false, revision = -1;
      const receive = (value: unknown) => {
        if (!stopped && validCapturePreferences(value) && value.revision >= revision) { revision = value.revision; accept(value); }
      };
      const stop = await transport.listen('capture-preferences-changed', receive);
      try { receive(await getCapturePreferences()); }
      catch (error) { stopped = true; stop(); throw error; }
      return () => { stopped = true; stop(); };
    },
    async info() {
      const value = await transport.invoke('settings_info');
      if (!validSettingsInfo(value)) throw new Error('软件信息无效');
      return value;
    },
    async openDataDirectory() { await transport.invoke('open_data_directory'); },
    async dataUsage() {
      const value = await transport.invoke('get_data_usage');
      if (!validDataUsage(value)) throw new Error('数据占用状态无效');
      return value;
    },
    async cleanData(category: DataCategory, expectedToken: string) {
      if (!validDataCategory(category) || !/^[0-9a-f]{64}$/.test(expectedToken)) throw new Error('清理范围无效');
      const value = await transport.invoke('clean_data', { category, expectedToken });
      if (!validDataCleanupReceipt(value)) throw new Error('清理结果无效，请刷新占用');
      return value;
    },
    async dataDirectoryState() {
      const value = await transport.invoke('get_data_directory_state');
      if (!validDataDirectoryState(value)) throw new Error('数据目录状态无效');
      return value;
    },
    async chooseDataDirectory() {
      const value = await transport.invoke('choose_data_directory');
      if (value === null) return null;
      if (!validDataDirectoryProposal(value)) throw new Error('所选数据目录无效');
      return value;
    },
    async cancelDataDirectoryProposal(proposalId: string) { await transport.invoke('cancel_data_directory_proposal', { proposalId }); },
    async migrateDataDirectory(proposal: DataDirectoryProposal) {
      if (!validDataDirectoryProposal(proposal)) throw new Error('所选数据目录无效');
      await transport.invoke('migrate_data_directory', { proposalId: proposal.proposalId, expectedGeneration: proposal.generation });
    },
    async readLicenseDocument(kind: LicenseDocumentKind) {
      const value = await transport.invoke('read_license_document', { kind });
      if (!validLicenseDocument(value)) throw new Error('许可文档无效');
      return value;
    },
  };
}
export type SettingsClient = ReturnType<typeof settingsClient>;
