// SPDX-License-Identifier: MPL-2.0
export interface RecordingSelectionSource { sceneId: string; backgroundId: string }
interface Hooks {
  current(source: RecordingSelectionSource): boolean;
  changed(source: RecordingSelectionSource | undefined): void;
  start(source: RecordingSelectionSource, regionId: string): Promise<void>;
  error(error: unknown): void;
}
/** One presentation intent, never an authority to reuse an old screen's crop.
 * The native same-session lease still admits the eventual saved region. */
export class RecordingSelection {
  private source?: RecordingSelectionSource;
  private timer?: ReturnType<typeof setTimeout>;
  private epoch = 0;
  constructor(private readonly hooks: Hooks) {}
  arm(source: RecordingSelectionSource) {
    this.clear(); this.source = { ...source }; const retained = this.source;
    this.hooks.changed(retained);
    // This is only a bounded UI hint. The worker owns its earlier real deadline.
    this.timer = setTimeout(() => { if (this.source === retained) this.clear(); }, 120_000);
  }
  clear() {
    this.epoch++;
    clearTimeout(this.timer); this.timer = undefined;
    if (this.source) { this.source = undefined; this.hooks.changed(undefined); }
  }
  reconcile() { if (this.source && !this.hooks.current(this.source)) this.clear(); }
  completed(regionId: string): boolean {
    const source = this.source;
    if (!source) return false;
    this.clear();
    const epoch = this.epoch;
    if (!this.hooks.current(source)) return false;
    // Consume before awaiting, so a duplicate saved-region callback or R cannot
    // start twice. No fresh portal request is issued by this path.
    void Promise.resolve().then(() => {
      if (this.epoch === epoch && this.hooks.current(source)) return this.hooks.start(source, regionId);
    }).catch(error => { if (this.epoch === epoch && this.hooks.current(source)) this.hooks.error(error); });
    return true;
  }
}
