// SPDX-License-Identifier: MPL-2.0
// Pure lease policy shared by the GNOME adapter and its contract tests.
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const senderName = /^:\d+\.\d+$/;
export class Lease {
  constructor(now, release) { this.now = now; this.release = release; this.active = undefined; }
  expire() {
    const a = this.active, now = this.now();
    if (a && (now < a.touched || now - a.touched >= 10_000 || now - a.created >= 600_000)) this.clear();
  }
  clear() { if (this.active) { this.active = undefined; this.release(); } }
  arm(sender, token) {
    this.expire();
    if (!senderName.test(sender) || !uuid.test(token) || this.active) throw Error('lease unavailable');
    const now = this.now();
    this.active = { sender, token, created: now, touched: now, stopped: false };
  }
  snapshot(sender, token) {
    this.expire();
    const a = this.active;
    if (!a || a.sender !== sender || a.token !== token) throw Error('lease unavailable');
    a.touched = this.now();
    return { stopped: a.stopped };
  }
  stop() { if (this.active && !this.active.stopped) { this.active.stopped = true; this.release(); } }
  disarm(sender, token) {
    this.expire();
    if (!this.active) return;
    if (this.active.sender !== sender || this.active.token !== token) throw Error('lease unavailable');
    this.clear();
  }
  disconnected(sender) { if (this.active?.sender === sender) this.clear(); }
}

export function pointsFromRequest(text) {
  if (typeof text !== 'string' || text.length > 4096) throw Error('invalid request');
  const value = JSON.parse(text);
  if (!Array.isArray(value) || value.length > 16) throw Error('invalid request');
  return value.map(point => {
    if (!Array.isArray(point) || point.length !== 2 || !point.every(v => Number.isFinite(v) && Math.abs(v) <= 1_000_000)) throw Error('invalid point');
    return point;
  });
}

export function rectangle(value) {
  const result = [value.x, value.y, value.width, value.height];
  if (!result.every(Number.isSafeInteger) || result[2] <= 0 || result[3] <= 0 || result.some(v => Math.abs(v) > 1_000_000)) throw Error('invalid rectangle');
  return result;
}
