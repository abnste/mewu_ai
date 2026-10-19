// SPDX-License-Identifier: MPL-2.0
// Native HWND behavior is covered by the Windows tests in shared_capture_windows.
// This wiring gate prevents either production caller from showing its HUD first.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('scroll and recording attach the exact hidden HUD lease before their first show', async () => {
  const native = file => readFile(new URL(`../src-tauri/src/${file}`, import.meta.url), 'utf8');
  const shared = await native('shared_capture_windows.rs');
  const attach = shared.slice(shared.indexOf('pub fn attach_hud('), shared.indexOf('pub fn owns_hud('));
  const capture = attach.indexOf('let lease = Lease::capture(');
  const disable = attach.indexOf('lease.disable_hud_transitions_before_show()?');
  const publish = attach.indexOf('self.hud = Some(');
  assert.ok(capture >= 0 && capture < disable && disable < publish,
    'a real new lease must disable transitions successfully before the HUD is admitted');
  for (const file of ['scroll_host.rs', 'recording_controls_windows.rs']) {
    const source = await native(file);
    const attach = source.indexOf('session.attach_hud(&toolbar,');
    const show = source.indexOf('toolbar.show()');
    assert.ok(source.indexOf('.visible(false)') >= 0 && source.indexOf('.visible(false)') < attach,
      `${file} must create the HUD hidden`);
    assert.ok(attach >= 0 && show > attach, `${file} must attach before any first show`);
  }
});

test('only retained capture paths bind the original native main as the top-level HUD owner', async () => {
  const native = file => readFile(new URL(`../src-tauri/src/${file}`, import.meta.url), 'utf8');
  const recording = await native('recording_controls_windows.rs');
  const scrollFile = await native('scroll_host.rs');
  const scroll = scrollFile.slice(scrollFile.indexOf('async fn prepare_windows('), scrollFile.indexOf('fn ensure_preparing('));
  for (const [source, retained] of [[recording, 'retain_main'], [scroll, 'retained_owner']]) {
    assert.match(source, /Ok\(\(shared, (?:retain_main|retained_owner), main\)\)/);
    assert.match(source, /let \(shared, (?:retain_main|retained_owner), main\) = receive\.await/);
    assert.equal((source.match(/get_webview_window\("space"\)/g) || []).length, 1,
      'builder and later preparation must keep the original captured main, not a successor found by label');
    assert.ok(source.includes(`if ${retained} {\n            builder.owner(&main)`) || source.includes(`if ${retained} {\n        builder.owner(&main)`));
    assert.ok(source.includes('} else { builder };'), 'cloaked fallbacks must remain unowned');
    assert.ok(source.includes(`session.attach_hud(&toolbar, ${retained})`));
    assert.doesNotMatch(source, /\.parent(?:_raw)?\(/, 'an owned top-level HUD is not a child window');
    for (const show of source.matchAll(/toolbar\.show\(\)/g)) {
      const previous = source.slice(Math.max(0, show.index - 220), show.index);
      assert.ok(previous.includes('session.verify_hud_owner()'), 'every first show must recheck the pinned relationship');
    }
  }
  assert.match(recording, /let retain_main = session\.can_retain_main_outside\(\)\?/);
  assert.match(scroll, /let retained_owner = !automatic && session\.can_retain_main_outside\(\)\?/);
  const shared = await native('shared_capture_windows.rs');
  assert.match(shared, /if retained_owner && !self\.can_retain_main_outside\(\)\?/);
  assert.match(shared, /if \*retained_owner && self\.main\.prepared && !self\.retains_main_outside\(\)\?/);
});
