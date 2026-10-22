// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { Lease, pointsFromRequest, rectangle } from './policy.mjs';
const id = '1cd50d4e-64e0-46e7-afd4-a486047d739c';
test('stop is terminal; a second caller, token or Arm cannot replace the active native lease', () => {
  let now = 0, releases = 0;
  const lease = new Lease(() => now, () => releases++);
  lease.arm(':1.123', id);
  assert.throws(() => lease.arm(':1.123', id));
  assert.throws(() => lease.snapshot(':1.124', id));
  assert.throws(() => lease.disarm(':1.124', id));
  assert.throws(() => lease.snapshot(':1.123', 'other'));
  assert.equal(lease.snapshot(':1.123', id).stopped, false);
  lease.stop();
  lease.stop();
  assert.equal(releases, 1);
  now = 5000;
  assert.equal(lease.snapshot(':1.123', id).stopped, true);
  assert.throws(() => lease.arm(':1.123', id));
  lease.disarm(':1.123', id);
  assert.throws(() => lease.snapshot(':1.123', id));
});
test('idle expiry, hard deadline, clock rollback and bus disconnect release Escape', () => {
  for (const reason of ['idle', 'deadline', 'clock', 'disconnect']) {
    let now = 100, releases = 0;
    const lease = new Lease(() => now, () => releases++);
    lease.arm(':1.123', id);
    if (reason === 'idle') now += 10000;
    if (reason === 'deadline') { for (let i = 0; i < 599; i++) { now += 1000; lease.snapshot(':1.123', id); } now += 1000; }
    if (reason === 'clock') now--;
    if (reason === 'disconnect') { lease.disconnected(':1.124'); assert.ok(lease.active); lease.disconnected(':1.123'); }
    assert.throws(() => lease.snapshot(':1.123', id), reason);
    assert.equal(releases, 1, reason);
    lease.clear(); assert.equal(releases, 1);
  }
});
test('bounded metadata requests reject malformed points and unknown coordinate values', () => {
  assert.deepEqual(pointsFromRequest('[[-100,200],[1.5,2]]'), [[-100,200],[1.5,2]]);
  for (const value of ['null', '{}', '[null]', '[[1]]', '[[1,2,3]]', '[["1",2]]', '[[1e99,0]]', JSON.stringify(Array(17).fill([1,2]))]) assert.throws(() => pointsFromRequest(value));
  assert.deepEqual(rectangle({x:-1920,y:0,width:1920,height:1080}), [-1920,0,1920,1080]);
  for (const value of [{x:0,y:0,width:0,height:1},{x:0,y:0,width:1.5,height:1},{x:Infinity,y:0,width:1,height:1}]) assert.throws(() => rectangle(value));
});
