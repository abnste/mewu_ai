// SPDX-License-Identifier: MPL-2.0
import { sameCaptureValues, validCapturePreferences, type CapturePreferences, type CapturePreferenceValues } from './settings-contracts';
interface Draft { value: CapturePreferenceValues; expectedRevision: number }
export interface CapturePreferencesView { state?: CapturePreferences; draft?: Draft; values?: CapturePreferenceValues; pending: boolean; conflict: boolean; error: string }
const values = (value: CapturePreferenceValues): CapturePreferenceValues => ({ captureDelaySeconds: value.captureDelaySeconds, includeCursor: value.includeCursor, defaultImageFormat: value.defaultImageFormat });
const conflictMessage = '截图设置已变化，请载入最新设置';
export class CapturePreferencesController {
  private state?: CapturePreferences;
  private draft?: Draft;
  private flight?: Promise<void>;
  private error = '';
  private disposed = false;
  constructor(private save: (revision: number, value: CapturePreferenceValues) => Promise<CapturePreferences>, private changed: (view: CapturePreferencesView) => void) {}
  view(): CapturePreferencesView { return { state: this.state, draft: this.draft, values: this.draft?.value ?? this.state, pending: Boolean(this.flight), conflict: Boolean(this.state && this.draft && this.state.revision !== this.draft.expectedRevision), error: this.error }; }
  private emit() { if (!this.disposed) this.changed(this.view()); }
  accept(state: CapturePreferences) {
    if (this.disposed || !validCapturePreferences(state) || this.state && state.revision < this.state.revision) return;
    this.state = { ...state }; this.emit();
  }
  failed(error: unknown) { if (!this.disposed) { this.error = error instanceof Error ? error.message : String(error); this.emit(); } }
  loaded(state: CapturePreferences) { if (!this.draft) this.error = ''; this.accept(state); }
  edit(patch: Partial<CapturePreferenceValues>) {
    if (this.disposed || !this.state) return;
    const draft = this.draft ?? { value: values(this.state), expectedRevision: this.state.revision };
    const value = { ...draft.value, ...patch };
    if (!validCapturePreferences({ version: 1, revision: draft.expectedRevision, ...value })) return;
    this.draft = { ...draft, value }; this.error = ''; this.emit();
  }
  discard() { if (!this.flight) { this.draft = undefined; this.error = ''; this.emit(); } }
  loadLatest(state: CapturePreferences, expectedDraft: Draft | undefined) {
    if (this.disposed || this.flight || !validCapturePreferences(state)) return;
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
      if (!validCapturePreferences(receipt) || !sameCaptureValues(receipt, draft.value) || receipt.revision < draft.expectedRevision || receipt.revision > draft.expectedRevision + 1) throw new Error('截图设置保存回执无效');
      this.accept(receipt);
      // Only this call's receipt can advance a draft's CAS baseline. Never use
      // a newer global event to silently authorize overwriting another window.
      if (!this.state || this.state.revision !== receipt.revision || !sameCaptureValues(this.state, receipt)) { this.error = conflictMessage; return; }
      if (this.draft === draft) this.draft = undefined;
      else if (this.draft?.expectedRevision === draft.expectedRevision) this.draft = { ...this.draft, expectedRevision: receipt.revision };
    }).catch(error => { this.failed(error); }).finally(() => { this.flight = undefined; this.emit(); });
    this.emit(); return this.flight;
  }
  dispose() { this.disposed = true; }
}
