// SPDX-License-Identifier: MPL-2.0
import { sameSystemValues, validSystemPreferences, type SystemPreferences, type SystemPreferenceValues } from './settings-contracts';
interface Draft { value: SystemPreferenceValues; expectedRevision: number }
export interface SystemPreferencesView { state?: SystemPreferences; draft?: Draft; values?: SystemPreferenceValues; pending: boolean; conflict: boolean; error: string }
const values = (value: SystemPreferenceValues): SystemPreferenceValues => ({ networkProxyMode: value.networkProxyMode, networkProxyUrl: value.networkProxyUrl, launchAtStartup: 'startupRegistered' in value ? (value as SystemPreferences).startupRegistered : value.launchAtStartup, allowScreenShare: value.allowScreenShare, autoGenerateTitle: value.autoGenerateTitle });
const conflictMessage = '系统设置已变化，请载入最新设置';
export class SystemPreferencesController {
  private state?: SystemPreferences;
  private draft?: Draft;
  private flight?: Promise<void>;
  private error = '';
  private disposed = false;
  constructor(private save: (revision: number, value: SystemPreferenceValues) => Promise<SystemPreferences>, private changed: (view: SystemPreferencesView) => void) {}
  view(): SystemPreferencesView { return { state: this.state, draft: this.draft, values: this.draft?.value ?? this.state, pending: Boolean(this.flight), conflict: Boolean(this.state && this.draft && this.state.revision !== this.draft.expectedRevision), error: this.error }; }
  private emit() { if (!this.disposed) this.changed(this.view()); }
  accept(state: SystemPreferences) {
    if (this.disposed || !validSystemPreferences(state) || this.state && state.revision < this.state.revision) return;
    this.state = { ...state }; this.emit();
  }
  failed(error: unknown) { if (!this.disposed) { this.error = error instanceof Error ? error.message : String(error); this.emit(); } }
  loaded(state: SystemPreferences) { if (!this.draft) this.error = ''; this.accept(state); }
  edit(patch: Partial<SystemPreferenceValues>) {
    if (this.disposed || !this.state) return;
    const draft = this.draft ?? { value: values(this.state), expectedRevision: this.state.revision };
    const value = { ...draft.value, ...patch };
    if (!validSystemPreferences({ version: 1, revision: draft.expectedRevision, startupRegistered: this.state.startupRegistered, ...value })) return;
    this.draft = { ...draft, value }; this.error = ''; this.emit();
  }
  discard() { if (!this.flight) { this.draft = undefined; this.error = ''; this.emit(); } }
  loadLatest(state: SystemPreferences, expectedDraft: Draft | undefined) {
    if (this.disposed || this.flight || !validSystemPreferences(state)) return;
    this.accept(state);
    if (this.draft === expectedDraft) { this.draft = undefined; this.error = ''; this.emit(); }
  }
  saveDraft(): Promise<void> {
    if (this.flight) return this.flight;
    if (this.disposed || !this.draft || !this.state) return Promise.resolve();
    const draft = this.draft;
    if (draft.expectedRevision !== this.state.revision) { this.failed(conflictMessage); return Promise.resolve(); }
    this.error = '';
    this.flight = Promise.resolve().then(() => this.save(draft.expectedRevision, draft.value)).then(receipt => {
      if (this.disposed) return;
      if (!validSystemPreferences(receipt) || !sameSystemValues(receipt, draft.value) || receipt.startupRegistered !== draft.value.launchAtStartup || receipt.revision < draft.expectedRevision || receipt.revision > draft.expectedRevision + 1) throw new Error('系统设置保存回执无效');
      this.accept(receipt);
      // Only this call's receipt can advance a draft's CAS baseline. Never use
      // a newer global event to silently authorize overwriting another window.
      if (!this.state || this.state.revision !== receipt.revision || !sameSystemValues(this.state, receipt) || this.state.startupRegistered !== receipt.startupRegistered) { this.error = conflictMessage; return; }
      if (this.draft === draft) this.draft = undefined;
      else if (this.draft?.expectedRevision === draft.expectedRevision) this.draft = { ...this.draft, expectedRevision: receipt.revision };
    }).catch(error => { this.failed(error); }).finally(() => { this.flight = undefined; this.emit(); });
    this.emit(); return this.flight;
  }
  dispose() { this.disposed = true; }
}
