// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
const source = await readFile(new URL('./capture-display.ts', import.meta.url), 'utf8');
const { captureDisplayGeometry } = await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(source, { mode: 'transform' })).toString('base64')}`);
const asset = (width, height, space = 'physicalPixels') => ({ captureCoordinates: { space, originX: -3440, originY: -100, width, height } });
test('physical fullscreen screenshots remain exactly one source pixel per display pixel across fractional DPI', () => {
  for (const deviceScale of [1, 1.25, 1.5, 1.75, 2]) {
    const width = 3440, height = 1440;
    const geometry = captureDisplayGeometry(asset(width, height), width, height,
      { width: Math.round(width / deviceScale), height: Math.round(height / deviceScale) }, deviceScale);
    assert.equal(geometry.nativePixels, true);
    assert.equal(geometry.x, 0); assert.equal(geometry.y, 0);
    assert.equal(geometry.scale, 1 / deviceScale);
    assert.ok(Math.abs(geometry.width * deviceScale - width) < 1e-9);
    assert.ok(Math.abs(geometry.height * deviceScale - height) < 1e-9);
    assert.ok(Math.abs(100 * geometry.scale * deviceScale - 100) < 1e-9);
  }
});
test('imports, resized windows and Quartz captures preserve normal contain behavior', () => {
  for (const background of [undefined, asset(1920, 1080, 'quartzPoints'), asset(1920, 1080)]) {
    const geometry = captureDisplayGeometry(background, 1920, 1080, { width: 1000, height: 800 }, 1.25);
    assert.equal(geometry.nativePixels, false);
    assert.ok(Math.abs(geometry.width - 1000) < 1e-9); assert.ok(Math.abs(geometry.height - 562.5) < 1e-9);
    assert.ok(Math.abs(geometry.x) < 1e-9); assert.ok(Math.abs(geometry.y - 118.75) < 1e-9);
  }
});
test('a long screenshot, bad scale or missing physical extent never acquires native-pixel presentation', () => {
  for (const [background, deviceScale] of [[asset(1920, 1080), 1], [asset(1920, 2400), NaN], [asset(1920, 2400), 0]]) {
    const geometry = captureDisplayGeometry(background, 1920, 2400, { width: 1920, height: 2400 }, deviceScale);
    assert.equal(geometry.nativePixels, false);
  }
});
