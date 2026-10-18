// SPDX-License-Identifier: MPL-2.0
/** A command receipt is small; its authoritative snapshot travels on the
 * existing subscription. Observe that publication, including event-before-reply,
 * and recover a missed event with the ordinary full query. Never guess state. */
export class PublishedRevision {
  private listeners = new Set<() => void>();
  private stopped = false;
  constructor(private readonly current: () => number | undefined,
    private readonly refresh: () => Promise<void>, private readonly timeout = 1000) {}
  changed(): void { for (const notify of [...this.listeners]) notify(); }
  dispose(): void { this.stopped = true; this.changed(); }
  wait(revision: number, witnessed: () => boolean = () => true): Promise<void> {
    if (!Number.isSafeInteger(revision) || revision < 1) return Promise.reject(Error('保存回执无效'));
    if (this.stopped) return Promise.reject(Error('窗口已关闭'));
    if ((this.current() ?? -1) >= revision && witnessed()) return Promise.resolve();
    return new Promise((resolve, reject) => {
      let finished = false;
      const finish = (error?: unknown) => {
        if (finished) return;
        finished = true; clearTimeout(timer); this.listeners.delete(check);
        if (error) reject(error); else resolve();
      };
      const check = () => {
        if (this.stopped) finish(Error('窗口已关闭'));
        else if ((this.current() ?? -1) >= revision && witnessed()) finish();
      };
      const timer = setTimeout(async () => {
        try {
          if (this.stopped) { check(); return; }
          await this.refresh();
          check();
          if (!finished) finish(Error('未收到保存结果，请重试'));
        } catch (error) { finish(error); }
      }, this.timeout);
      this.listeners.add(check);
      check();
    });
  }
}
