// SPDX-License-Identifier: MPL-2.0
// Validate every vendored source file; no network, execution or audit suppression.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const provenance = JSON.parse(fs.readFileSync(path.join(root, 'docs/glib-backport.json'), 'utf8'));
const base = path.join(root, 'vendor/glib-0.18.5');
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const actual = [];
function walk(directory) {
  for (const item of fs.readdirSync(directory, { withFileTypes: true })) {
    const full = path.join(directory, item.name);
    if (item.isDirectory()) walk(full);
    else if (item.isFile()) actual.push(path.relative(base, full).split(path.sep).join('/'));
    else throw new Error('Vendored GLib contains a non-regular entry');
  }
}
walk(base);
const expected = Object.keys(provenance.upstreamFiles).sort();
if (JSON.stringify(actual.sort()) !== JSON.stringify(expected)) throw new Error('Vendored GLib file inventory changed');
for (const relative of expected) {
  const bytes = fs.readFileSync(path.join(base, relative));
  const hash = relative === provenance.modifiedFile ? provenance.patchedFileSha256 : provenance.upstreamFiles[relative];
  if (sha256(bytes) !== hash) throw new Error(`GLib source differs from recorded patch: ${relative}`);
  if (relative === provenance.modifiedFile) {
    const text = bytes.toString('utf8');
    const original = text.replace('let mut p: *mut libc::c_char', 'let p: *mut libc::c_char')
      .replace('                &mut p,\n', '                &p,\n');
    if (sha256(original) !== provenance.upstreamFiles[relative]) throw new Error('GLib patch is not the two-line upstream backport');
  }
}
console.log(`GLib ${provenance.version}: ${expected.length} source files verified; upstream PR 1343 backport only`);
