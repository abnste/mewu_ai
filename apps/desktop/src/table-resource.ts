// SPDX-License-Identifier: MPL-2.0
import type { TableMessage } from './table-contracts';

export async function tableTextHash(text: string) {
  const hash = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
  return [...new Uint8Array(hash)].map(value => value.toString(16).padStart(2, '0')).join('');
}

export class TableMessageCache {
  private entries = new Map<string, { promise: Promise<TableMessage>; bytes: number; settled: boolean }>();
  private running = 0;
  private queue: (() => void)[] = [];
  constructor(private capacity = 32, private maxBytes = 8 * 1024 * 1024, private concurrency = 2) {}
  get(key: string, load: () => Promise<TableMessage>): Promise<TableMessage> {
    const existing = this.entries.get(key);
    if (existing) { this.entries.delete(key); this.entries.set(key, existing); return existing.promise; }
    if (this.entries.size >= this.capacity) {
      const oldest = [...this.entries].find(([, value]) => value.settled);
      if (oldest) this.entries.delete(oldest[0]); else return Promise.reject(new Error('表格读取繁忙，请重试'));
    }
    const promise = new Promise<TableMessage>((resolve, reject) => {
      const start = () => {
        this.running++;
        Promise.resolve().then(load).then(resolve, reject).finally(() => { this.running--; this.queue.shift()?.(); });
      };
      if (this.running < this.concurrency) start(); else this.queue.push(start);
    });
    const entry = { promise, bytes: 0, settled: false }; this.entries.set(key, entry);
    void promise.then(value => {
      entry.settled = true; entry.bytes = JSON.stringify(value).length * 2;
      let total = [...this.entries.values()].reduce((sum, value) => sum + value.bytes, 0);
      for (const [key, value] of this.entries) {
        if (total <= this.maxBytes) break;
        if (value.settled) { total -= value.bytes; this.entries.delete(key); }
      }
    }, () => { if (this.entries.get(key) === entry) this.entries.delete(key); });
    return promise;
  }
}

export class TableRequestGate {
  private version = 0;
  private disposed = false;
  cancel() { this.version++; }
  dispose() { this.disposed = true; this.cancel(); }
  async load(request: () => Promise<TableMessage>, accept: (value: TableMessage) => void, error: (error: unknown) => void) {
    const version = ++this.version;
    try { const value = await request(); if (!this.disposed && version === this.version) accept(value); }
    catch (value) { if (!this.disposed && version === this.version) error(value); }
  }
}
