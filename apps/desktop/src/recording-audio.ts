// SPDX-License-Identifier: MPL-2.0
import type { PluginRecord } from './plugin-contracts';

export type RecordingAudioMode = 'mute' | 'system' | 'microphone' | 'both';
export interface RecordingGrant { pluginId: string; revision: number; contributionId: string }
export interface RecordingAudioGrant { pluginId: string; revision: number; contributionId: string }
export interface RecordingAudioSelection { revision: number; mode: RecordingAudioMode; grant: RecordingAudioGrant | null }
export interface RecordingAudioState extends RecordingAudioSelection { sequence: number; editable: boolean; message?: string }
export interface RecordingAudioDraft { mode: RecordingAudioMode; grant: RecordingAudioGrant | null }
export interface RecordingAudioView { state?: RecordingAudioState; mode: RecordingAudioMode; draft?: RecordingAudioDraft; pending: boolean; error: string }
export const audioModes: RecordingAudioMode[] = ['mute', 'system', 'microphone', 'both'];
export const audioLabels: Record<RecordingAudioMode, string> = { mute: '静音', system: '电脑声', microphone: '麦克风', both: '两者' };
export const sameAudioGrant = (a: RecordingAudioGrant | null, b: RecordingAudioGrant | null) => a === b || Boolean(a && b && a.pluginId === b.pluginId && a.revision === b.revision && a.contributionId === b.contributionId);
export const sameAudioSelection = (a: RecordingAudioSelection, b: RecordingAudioSelection) => a.revision === b.revision && a.mode === b.mode && sameAudioGrant(a.grant, b.grant);
const coreRecordingId = 'mewu.core.recording';
export const recordingGrant = (_plugins: PluginRecord[] = []): RecordingGrant => ({ pluginId: coreRecordingId, revision: 1, contributionId: 'record' });
export const recordingTrimGrant = (_plugins: PluginRecord[] = []): RecordingGrant => ({ pluginId: coreRecordingId, revision: 1, contributionId: 'trim' });
export const recordingGifGrant = (): RecordingGrant => ({ pluginId: coreRecordingId, revision: 1, contributionId: 'gif' });
export function recordingAudioGrants(_plugins: PluginRecord[] = []): RecordingAudioGrant[] {
  return [{ pluginId: coreRecordingId, revision: 1, contributionId: 'audio' }];
}
export function validRecordingTrimGrant(value: RecordingGrant): boolean {
  return sameAudioGrant(value, recordingTrimGrant());
}
export function validRecordingAudioState(value: unknown): value is RecordingAudioState {
  if (!value || typeof value !== 'object') return false;
  const v = value as RecordingAudioState, g = v.grant;
  return Number.isSafeInteger(v.revision) && v.revision >= 0 && Number.isSafeInteger(v.sequence) && v.sequence >= 0 && typeof v.editable === 'boolean' && audioModes.includes(v.mode) && (v.message === undefined || typeof v.message === 'string') && (v.mode === 'mute' ? g === null : Boolean(g && typeof g.pluginId === 'string' && g.pluginId.length && Number.isSafeInteger(g.revision) && g.revision > 0 && typeof g.contributionId === 'string' && g.contributionId.length));
}

/** A failed selection stays a draft; only a native receipt may authorize capture. */
export class RecordingAudioController {
  private state?: RecordingAudioState;
  private draft?: RecordingAudioDraft;
  private grants?: RecordingAudioGrant[];
  private flight?: Promise<void>;
  private error = '';
  private disposed = false;
  constructor(private save: ((expectedRevision: number, mode: RecordingAudioMode, grant: RecordingAudioGrant | null) => Promise<RecordingAudioState>) | undefined, private changed: (view: RecordingAudioView) => void) {}
  private allowed(grant: RecordingAudioGrant | null) { return Boolean(grant && this.grants?.some(value => sameAudioGrant(value, grant))); }
  private needsSelection() { return Boolean(this.state && this.state.mode !== 'mute' && !this.allowed(this.state.grant)); }
  private effective(): RecordingAudioSelection | undefined {
    const state = this.state; if (!state) return;
    if (this.needsSelection()) return;
    return { revision: state.revision, mode: state.mode, grant: state.grant && { ...state.grant } };
  }
  view(): RecordingAudioView { return { state: this.state, mode: this.draft?.mode ?? this.state?.mode ?? 'system', draft: this.draft, pending: Boolean(this.flight), error: this.error || (this.needsSelection() ? '请重新选择声源' : '') }; }
  private emit() { if (!this.disposed) this.changed(this.view()); }
  accept(value: RecordingAudioState) {
    if (this.disposed || !validRecordingAudioState(value) || this.state && value.sequence < this.state.sequence) return;
    this.state = structuredClone(value); this.emit();
  }
  reconcile(grants: RecordingAudioGrant[]) {
    this.grants = grants;
    if (this.draft?.mode !== 'mute' && this.draft && !this.allowed(this.draft.grant)) { this.draft = undefined; this.error = ''; }
    this.emit();
  }
  readFailed(error: unknown) { this.error = error instanceof Error ? error.message : String(error); this.emit(); }
  readSucceeded(state: RecordingAudioState) { if (!this.draft) this.error = ''; this.accept(state); }
  discard() { if (!this.flight) { this.draft = undefined; this.error = ''; this.emit(); } }
  choose(mode: RecordingAudioMode, grant: RecordingAudioGrant | null): Promise<void> {
    if (this.disposed || this.flight) return this.flight ?? Promise.resolve();
    const save = this.save;
    if (!save) { this.readFailed('声音设置尚未就绪'); return Promise.resolve(); }
    if (!this.state?.editable) { this.readFailed(this.state?.message || '声音设置尚未就绪'); return Promise.resolve(); }
    const draft: RecordingAudioDraft = { mode, grant: mode === 'mute' ? null : grant && { ...grant } };
    if (mode !== 'mute' && !this.allowed(draft.grant)) { this.readFailed('录音不可用'); return Promise.resolve(); }
    this.draft = draft; this.error = '';
    const revision = this.state.revision;
    this.flight = Promise.resolve().then(() => save(revision, draft.mode, draft.grant)).then(receipt => {
      if (this.disposed) return;
      this.accept(receipt);
      if (this.draft !== draft) return;
      const current = this.effective();
      if (current && current.revision === receipt.revision && current.mode === draft.mode && sameAudioGrant(current.grant, draft.grant)) this.draft = undefined;
      else this.error = '声音设置已变化，请重新选择';
    }, error => { if (!this.disposed && this.draft === draft) this.error = error instanceof Error ? error.message : String(error); }).finally(() => { this.flight = undefined; this.emit(); });
    this.emit(); return this.flight;
  }
  async prepare(): Promise<RecordingAudioSelection> { if (this.flight) await this.flight; return this.selection(); }
  selection(): RecordingAudioSelection {
    if (this.disposed || this.flight || !this.state || !this.grants) throw new Error('声音设置尚未就绪');
    if (this.draft) throw new Error(this.error || '请先保存或取消声音设置');
    const selection = this.effective();
    if (!selection) throw new Error('请重新选择声源');
    return selection;
  }
  dispose() { this.disposed = true; }
}
