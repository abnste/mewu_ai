// SPDX-License-Identifier: MPL-2.0
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule } from 'node:vm';
const module = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./capture-shortcut-portal.ts', import.meta.url), 'utf8'), { mode: 'transform' }));
await module.link(() => { throw Error('unexpected runtime import'); }); await module.evaluate();
const { portalShortcutHasDisableAction, portalShortcutLabel, ShortcutPortalController } = module.namespace;
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const tick = () => new Promise(resolve => setImmediate(resolve));
const state = (extra = {}) => ({ revision: 3, sequence: 4, configured: { code: 'KeyS', ctrl: false, shift: true, alt: true }, active: null, status: 'needs_setup', editable: true, backend: 'wayland_portal', ...extra });
function fixture() {
  const requests = [], cancellations = [], views = [], accepted = [], errors = []; let allowed = true, serial = 0;
  const controller = new ShortcutPortalController({ allowed: () => allowed, id: () => `request-${++serial}`, configure: (revision, id) => { const wait = deferred(); requests.push({ revision, id, ...wait }); return wait.promise; }, cancel: id => { const wait = deferred(); cancellations.push({ id, ...wait }); return wait.promise; }, changed: view => views.push(view), accepted: value => accepted.push(value), error: error => errors.push(error) });
  return { controller, requests, cancellations, views, accepted, errors, allowed: value => { allowed = value; } };
}

test('only an active portal response label is presented, never configured/default chord or a native backend hint', () => {
  for (const status of ['needs_setup', 'disabled', 'unavailable', 'cleanup_pending']) assert.equal(portalShortcutLabel(state({ status, actualTriggerDescription: 'Preferred shortcut' })), undefined);
  for (const value of [null, '', ' \t ']) assert.equal(portalShortcutLabel(state({ status: 'active', actualTriggerDescription: value })), undefined);
  const label = 'Super + Capture (system-defined)';
  assert.equal(portalShortcutLabel(state({ status: 'active', actualTriggerDescription: label })), label);
  assert.equal(portalShortcutLabel(state({ backend: 'native', status: 'active', actualTriggerDescription: label })), undefined);
  assert.equal(portalShortcutLabel(undefined), undefined);
});
test('first-time setup has no redundant disable action; active, previously enabled or cleanup debt can be disabled', () => {
  assert.equal(portalShortcutHasDisableAction(state()), false);
  assert.equal(portalShortcutHasDisableAction(state({ portalEnabled: false })), false);
  assert.equal(portalShortcutHasDisableAction(state({ portalEnabled: true })), true);
  for (const status of ['active', 'cleanup_pending']) assert.equal(portalShortcutHasDisableAction(state({ status, portalEnabled: false })), true);
  assert.equal(portalShortcutHasDisableAction(state({ status: 'disabled', portalEnabled: true })), false);
  assert.equal(portalShortcutHasDisableAction(state({ backend: 'native', status: 'active' })), false);
  assert.equal(portalShortcutHasDisableAction(undefined), false);
});

test('no configure on load/disallowed scope; an explicit configure owns one nonce and CAS revision', async () => {
  const f = fixture(); assert.equal(f.requests.length, 0);
  f.allowed(false); await f.controller.configure(7); assert.equal(f.requests.length, 0);
  f.allowed(true); const first = f.controller.configure(7); await f.controller.configure(8);
  assert.equal(f.requests.length, 1); assert.deepEqual([f.requests[0].revision, f.requests[0].id], [7, 'request-1']);
  const receipt = state({ status: 'active', actualTriggerDescription: 'Ctrl+Space' });
  f.requests[0].resolve(receipt); await first; assert.equal(f.accepted[0], receipt); assert.equal(f.views.at(-1).requestId, undefined);
  f.controller.dispose(); assert.equal(f.cancellations.length, 0);
});

test('cancel has an independent RPC while consent is pending; old result cannot overwrite the next request', async () => {
  const f = fixture(), first = f.controller.configure(7), cancel = f.controller.cancel();
  assert.equal(f.controller.cancel(), cancel); await tick();
  assert.equal(f.cancellations.length, 1); assert.equal(f.cancellations[0].id, 'request-1'); assert.equal(f.views.at(-1).canceling, true);
  f.cancellations[0].resolve(); await cancel; assert.equal(f.views.at(-1).requestId, undefined);
  const second = f.controller.configure(8); assert.equal(f.requests.length, 2);
  f.requests[0].resolve(state({ status: 'active', actualTriggerDescription: 'old' })); await first;
  assert.equal(f.accepted.length, 0); assert.equal(f.views.at(-1).requestId, 'request-2');
  const next = state({ revision: 8, status: 'active', actualTriggerDescription: 'new' }); f.requests[1].resolve(next); await second;
  assert.deepEqual(f.accepted, [next]); assert.equal(f.views.at(-1).requestId, undefined); f.controller.dispose();
});

test('failed cancellation retains the exact cleanup nonce and prevents another configuration until retry acknowledgement', async () => {
  const f = fixture(), first = f.controller.configure(7), cancel = f.controller.cancel(); await tick();
  f.cancellations[0].reject(Error('cancel failed')); await assert.rejects(cancel, /cancel failed/);
  assert.deepEqual(f.views.at(-1), { requestId: 'request-1', canceling: false, configuring: false }); await f.controller.configure(8); assert.equal(f.requests.length, 1);
  f.requests[0].reject(Error('late consent failed')); await first; assert.equal(f.errors.length, 0);
  const retry = f.controller.cancel(); await tick(); assert.equal(f.cancellations[1].id, 'request-1');
  f.cancellations[1].resolve(); await retry; const second = f.controller.configure(8); f.requests[1].resolve(state()); await second; f.controller.dispose();
});

test('disposal cancels only the owned nonce and suppresses late receipts, errors, and UI updates', async () => {
  for (const fail of [false, true]) {
    const f = fixture(), first = f.controller.configure(7); f.controller.dispose(); const count = f.views.length; await tick();
    assert.equal(f.cancellations.length, 1); assert.equal(f.cancellations[0].id, 'request-1');
    if (fail) f.requests[0].reject(Error('late')); else f.requests[0].resolve(state());
    f.cancellations[0].resolve(); await first; await tick(); await f.controller.configure(8);
    assert.equal(f.accepted.length, 0); assert.equal(f.errors.length, 0); assert.equal(f.views.length, count); assert.equal(f.requests.length, 1);
  }
});

test('visible configure failures report once and release the UI for an explicit retry', async () => {
  const f = fixture(), first = f.controller.configure(7); f.requests[0].reject(Error('denied')); await first;
  assert.equal(f.errors.length, 1); assert.equal(f.views.at(-1).requestId, undefined);
  const second = f.controller.configure(7); assert.equal(f.requests[1].id, 'request-2'); f.requests[1].resolve(state()); await second; f.controller.dispose();
});

test('a native recovery nonce can be canceled without configuring; local owner beats stale published nonce', async () => {
  const f = fixture(), canceled = f.controller.cancelPublished('native-recovery'); await tick();
  assert.equal(f.requests.length, 0); assert.equal(f.cancellations[0].id, 'native-recovery'); assert.equal(f.views.at(-1).configuring, false);
  f.cancellations[0].resolve(); await canceled;
  const pending = f.controller.configure(7), cancel = f.controller.cancelPublished('stale-recovery'); await tick();
  assert.equal(f.cancellations[1].id, 'request-1'); f.cancellations[1].resolve(); await cancel;
  f.requests[0].resolve(state()); await pending; assert.equal(f.accepted.length, 0);
  f.controller.dispose(); await f.controller.cancelPublished('late'); assert.equal(f.cancellations.length, 2);
});
