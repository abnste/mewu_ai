// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { createContext, SourceTextModule } from 'node:vm';
import { test } from 'node:test';
const module = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./video-preview.ts', import.meta.url), 'utf8'), { mode: 'transform' }), { context: createContext({}) });
await module.link(() => { throw new Error('unexpected import'); }); await module.evaluate();
const { previewSize, reducedVideoPreview } = module.namespace;
test('scaled preview dimensions preserve source/export geometry and reject invalid dimensions', () => {
  for (const percent of [50, 75, 100]) assert.equal(previewSize(1920, 1080, percent).width, 1920 * percent / 100);
  assert.equal(previewSize(1, 1, 50).width, 1);
  for (const width of [0, NaN, Infinity, -1, 8193, 1.5]) assert.equal(previewSize(width, 1080, 50), undefined);
});
test('real media clock and seek are retained; drawing stops and releases surface after disposal', () => {
  const listeners = new Map(), callbacks = new Map(), cancelled = [], drawn = [];
  let seq = 0, visible = false;
  const video = { src: 'original.mp4', currentTime: 4.25, readyState: 2, paused: false, ended: false, videoWidth: 1920, videoHeight: 1080,
    addEventListener: (name, callback) => listeners.set(name, callback), removeEventListener: name => listeners.delete(name),
    requestVideoFrameCallback: cb => { callbacks.set(++seq, cb); return seq; }, cancelVideoFrameCallback: id => { cancelled.push(id); callbacks.delete(id); } };
  const canvas = { width: 1, height: 1, getContext: () => ({ drawImage: (...v) => drawn.push(v) }) };
  const stop = reducedVideoPreview(video, canvas, 50, v => visible = v);
  assert.equal(visible, true); assert.equal(canvas.width, 960); assert.equal(canvas.height, 540);
  assert.equal(video.src, 'original.mp4'); assert.equal(video.currentTime, 4.25); assert.equal(callbacks.size, 1);
  const frame = callbacks.get(1); callbacks.delete(1); frame();
  assert.equal(drawn.length, 2); assert.equal(callbacks.size, 1);
  video.paused = true; video.currentTime = 6; listeners.get('seeked')();
  assert.equal(drawn.length, 3); assert.equal(callbacks.size, 1);
  const late = [...callbacks.values()][0]; stop(); late();
  assert.equal(drawn.length, 3); assert.equal(listeners.size, 0); assert.equal(visible, false); assert.equal(canvas.width, 1); assert.equal(cancelled.length, 1);
});
test('original resolution has no extra frame copy; failed canvas keeps native video visible', () => {
  let requested = 0, visible = false;
  const video = { readyState: 2, paused: true, videoWidth: 320, videoHeight: 180, addEventListener() {}, removeEventListener() {} };
  const canvas = { getContext: () => { requested++; return { drawImage: () => { throw new Error('device lost'); } }; } };
  reducedVideoPreview(video, canvas, 100, v => visible = v)(); assert.equal(requested, 0);
  reducedVideoPreview(video, canvas, 50, v => visible = v)(); assert.equal(requested, 1); assert.equal(visible, false);
});

test('Mac full-resolution presentation keeps the source clock and redraws paused seek, then cancels late frames', () => {
  const listeners = new Map(), callbacks = new Map(), drawn = [], cancelled = [];
  let next = 0, visible = false;
  const video = { src: 'source.mp4', currentTime: 0, readyState: 1, paused: true, ended: false, videoWidth: 286, videoHeight: 268,
    addEventListener: (name, cb) => listeners.set(name, cb), removeEventListener: name => listeners.delete(name),
    requestVideoFrameCallback: cb => { callbacks.set(++next, cb); return next; }, cancelVideoFrameCallback: id => { cancelled.push(id); callbacks.delete(id); } };
  const canvas = { width: 1, height: 1, getContext: () => ({ drawImage: (...args) => drawn.push(args) }) };
  const stop = reducedVideoPreview(video, canvas, 100, value => visible = value, true);
  assert.equal(drawn.length, 0); assert.equal(visible, false);
  video.readyState = 2; listeners.get('loadeddata')();
  assert.equal(visible, true); assert.equal(canvas.width, 286); assert.equal(canvas.height, 268);
  assert.equal(callbacks.size, 0);
  video.currentTime = 2; listeners.get('seeked')();
  assert.equal(drawn.length, 2); assert.equal(video.currentTime, 2); assert.equal(video.src, 'source.mp4');
  video.paused = false; listeners.get('play')();
  assert.equal(callbacks.size, 1);
  const late = callbacks.get(1); stop(); late();
  assert.equal(drawn.length, 3); assert.equal(cancelled.length, 1); assert.equal(callbacks.size, 0);
  assert.equal(listeners.size, 0); assert.equal(visible, false);
});

test('immutable pinned decoder reload retains the last presented frame, and disposal still releases it', () => {
  const events = new Map(), frames = new Map(), draws = [], states = [];
  let next = 0;
  const video = { readyState: 2, paused: false, ended: false, videoWidth: 320, videoHeight: 180,
    addEventListener: (name, fn) => events.set(name, fn), removeEventListener: name => events.delete(name),
    requestVideoFrameCallback: fn => { frames.set(++next, fn); return next; }, cancelVideoFrameCallback: id => frames.delete(id) };
  const canvas = { width: 1, height: 1, getContext: () => ({ drawImage: () => draws.push(video.readyState) }) };
  const stop = reducedVideoPreview(video, canvas, 100, value => states.push(value), true, true);
  assert.equal(states.at(-1), true); assert.equal(draws.length, 1);
  video.readyState = 0; events.get('emptied')(); events.get('seeked')();
  assert.equal(states.at(-1), true); assert.equal(draws.length, 1); assert.equal(canvas.width, 320);
  video.readyState = 2; events.get('loadeddata')(); assert.equal(draws.length, 2);
  const late = [...frames.values()][0]; stop(); late();
  assert.equal(draws.length, 2); assert.equal(states.at(-1), false); assert.equal(canvas.width, 1);
  assert.equal(events.size, 0); assert.equal(frames.size, 0);
});
