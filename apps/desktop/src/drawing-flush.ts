// SPDX-License-Identifier: MPL-2.0
export type DrawingFlush = (active: () => boolean) => Promise<void>;
export type RegisterDrawingFlush = (flush: DrawingFlush) => () => void;

/** One mounted editor owns the handoff. An older cleanup cannot clear its successor. */
export class DrawingFlushRegistry {
  private current?: DrawingFlush;
  register: RegisterDrawingFlush = flush => {
    this.current = flush;
    return () => { if (this.current === flush) this.current = undefined; };
  };
  async flush(active: () => boolean): Promise<void> {
    if (!active()) return;
    const owner = this.current;
    if (!owner) return;
    await owner(active);
    if (active() && this.current !== owner) throw new Error('绘制会话已变化');
  }
}
