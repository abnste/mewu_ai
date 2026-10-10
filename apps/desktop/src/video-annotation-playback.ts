// SPDX-License-Identifier: MPL-2.0
// Loading may hold the current frame, never seek. Only this exact play intent
// can continue after the raster arrives; a user/system pause revokes it.
export class VideoAnnotationPlaybackHold {
  private generation = 0;
  private held?: { generation: number; source: string; intent?: number };
  invalidate() { this.generation++; this.held = undefined; }
  update(input: { source: string; loading: boolean; failed: boolean; playing: boolean; allowed: boolean; intent?: number }, pause: () => void, resume: () => Promise<void>, error: (cause: unknown) => void) {
    if (input.failed || !input.allowed || (this.held && (this.held.source !== input.source || this.held.intent !== input.intent))) { this.invalidate(); return; }
    if (input.loading) {
      if (input.playing && !this.held) { this.held = { generation: this.generation, source: input.source, intent: input.intent }; pause(); }
      return;
    }
    const held = this.held; if (!held) return;
    this.held = undefined;
    if (held.generation !== this.generation || held.source !== input.source) return;
    void resume().catch(cause => { if (held.generation === this.generation) error(cause); });
  }
}
