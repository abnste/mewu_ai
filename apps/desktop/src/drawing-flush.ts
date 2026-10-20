// SPDX-License-Identifier: MPL-2.0
export type DrawingFlush = ((active: () => boolean) => Promise<void>) & {
  /** Read existing authoring state without canceling or saving user input. */
  interacting?: () => boolean;
  controlTarget?: () => { sceneId: string; backgroundId: string };
};
export type RegisterDrawingFlush = (flush: DrawingFlush) => () => void;

/** One mounted editor owns the handoff. An older cleanup cannot clear its successor. */
export class DrawingFlushRegistry {
  private current?: DrawingFlush;
  register: RegisterDrawingFlush = flush => {
    this.current = flush;
    return () => { if (this.current === flush) this.current = undefined; };
  };
  interacting(): boolean {
    if (!this.current) return false;
    try { return this.current.interacting?.() !== false; }
    catch { return true; }
  }
  idle(sceneId: string, backgroundId: string | undefined): boolean {
    // A missing/old/unknown editor cannot turn a broad selection lock into
    // permission. Cleanup retains the same owner fence as the actual flush.
    const owner = this.current;
    if (!owner?.interacting || !owner.controlTarget || !backgroundId) return false;
    try {
      const target = owner.controlTarget();
      const active = owner.interacting();
      return this.current === owner && target.sceneId === sceneId && target.backgroundId === backgroundId && active === false;
    } catch { return false; }
  }
  async flush(active: () => boolean): Promise<void> {
    if (!active()) return;
    const owner = this.current;
    if (!owner) return;
    await owner(active);
    if (active() && this.current !== owner) throw new Error('绘制会话已变化');
  }
}
