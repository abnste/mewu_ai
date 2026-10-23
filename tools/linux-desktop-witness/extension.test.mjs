// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { SourceTextModule, SyntheticModule, createContext } from 'node:vm';
import { Lease, pointsFromRequest, rectangle } from './policy.mjs';

const token = '1cd50d4e-64e0-46e7-afd4-a486047d739c';
const epoch = '9e23a8c4-abca-4a21-9716-fef6b449f2ac';
async function fixture(flags = {}) {
  let now = 0, redraws = 0;
  const forbidden = () => { throw Error('no screenshot, focus or input interface'); };
  const context = createContext({ global: {
    stage: { queue_redraw() { redraws++; if (flags.redrawError) throw Error('unavailable'); }, get_actor_at_pos: forbidden },
    display: { is_grabbed: () => flags.grabbed ?? false }, get_pointer: forbidden, get_window_actors: forbidden,
  } });
  const glib = { Variant: class { constructor(signature, values) { this.signature = signature; this.values = values; } } };
  const main = { modalCount: flags.modal ?? 0, sessionMode: { isLocked: flags.locked ?? false }, overview: { visible: flags.overview ?? false } };
  const dependencies = {
    './policy.mjs': { Lease, pointsFromRequest, rectangle },
    'resource:///org/gnome/shell/extensions/extension.js': { Extension: class {} },
    'resource:///org/gnome/shell/ui/main.js': main,
    'gi://GLib': { default: glib },
  };
  const module = new SourceTextModule(await readFile(new URL('./extension.js', import.meta.url), 'utf8'), { context });
  await module.link(specifier => {
    const exports = dependencies[specifier] ?? { default: {} };
    return new SyntheticModule(Object.keys(exports), function () {
      for (const [name, value] of Object.entries(exports)) this.setExport(name, value);
    }, { context });
  });
  await module.evaluate();
  const instance = new module.namespace.default();
  instance._epoch = epoch;
  instance._lease = new Lease(() => now, () => {});
  instance._lease.arm(':1.123', token);
  return { instance, advance(value) { now += value; }, redraws: () => redraws,
    request(sender = ':1.123', suppliedToken = token) {
      let result, error;
      instance.RequestFrameAsync([suppliedToken], {
        get_sender: () => sender,
        return_value(value) { result = JSON.parse(value.values[0]); },
        return_dbus_error(name) { error = name; },
      });
      return { result, error };
    },
  };
}

test('production RequestFrame queues one repaint and returns the current lease epoch without reading pixels or injecting input', async () => {
  const f = await fixture();
  assert.deepEqual(f.request(), { result: { protocol: 1, epoch }, error: undefined });
  assert.equal(f.redraws(), 1);
});
test('production RequestFrame rejects a foreign caller or token before repaint', async () => {
  for (const args of [[':1.124', token], [':1.123', 'other']]) {
    const f = await fixture(); assert.ok(f.request(...args).error); assert.equal(f.redraws(), 0);
  }
});
test('production RequestFrame rejects stopped, expired and disconnected leases', async () => {
  for (const reason of ['stop', 'expiry', 'disconnect']) {
    const f = await fixture();
    if (reason === 'stop') f.instance._lease.stop();
    if (reason === 'expiry') f.advance(10000);
    if (reason === 'disconnect') f.instance._lease.disconnected(':1.123');
    assert.ok(f.request().error, reason); assert.equal(f.redraws(), 0, reason);
  }
});
test('production RequestFrame cannot repaint a locked, modal, grabbed or overview desktop', async () => {
  for (const flags of [{ locked: true }, { modal: 1 }, { grabbed: true }, { overview: true }]) {
    const f = await fixture(flags); assert.ok(f.request().error); assert.equal(f.redraws(), 0);
  }
});
test('production RequestFrame does not acknowledge a failed compositor repaint request', async () => {
  const f = await fixture({ redrawError: true }); const value = f.request();
  assert.ok(value.error); assert.equal(value.result, undefined);
});
