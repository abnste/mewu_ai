// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { inspectAppDir, normalizeAppDir, squashfsOffset, rebuildAppImage } from './normalize-linux-appimage.mjs';

test('ELF file ranges reject false filesystem magic, overflow, missing headers and wrong ABI', () => {
  const bytes = Buffer.alloc(256);
  bytes.set([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1]); bytes.set([0x41, 0x49, 2], 8);
  bytes.writeUInt16LE(2, 16); bytes.writeUInt16LE(62, 18); bytes.writeUInt32LE(1, 20);
  bytes.writeUInt16LE(64, 52); bytes.writeBigUInt64LE(64n, 32); bytes.writeUInt16LE(56, 54); bytes.writeUInt16LE(1, 56);
  bytes.writeBigUInt64LE(120n, 40); bytes.writeUInt16LE(64, 58); bytes.writeUInt16LE(1, 60);
  bytes.writeBigUInt64LE(184n, 72); bytes.writeBigUInt64LE(32n, 96);
  bytes.writeUInt32LE(1, 124); bytes.writeBigUInt64LE(184n, 144); bytes.writeBigUInt64LE(32n, 152);
  bytes.write('hsqs', 216);
  assert.equal(squashfsOffset(bytes), 216);
  const staleMagic = Buffer.from(bytes); staleMagic.write('hsqs', 200); staleMagic.fill(0, 216, 220);
  assert.throws(() => squashfsOffset(staleMagic), /boundary/);
  const overflow = Buffer.from(bytes); overflow.writeBigUInt64LE(2n ** 63n, 32);
  assert.throws(() => squashfsOffset(overflow), /offset/);
  const wrongArch = Buffer.from(bytes); wrongArch.writeUInt16LE(183, 18);
  assert.throws(() => squashfsOffset(wrongArch), /x86_64/);
  const noTable = Buffer.from(bytes); noTable.writeUInt16LE(0, 60);
  assert.throws(() => squashfsOffset(noTable), /table/);
  assert.throws(() => squashfsOffset(bytes.subarray(0, 32)), /bounded/);
});

test('native permissions preserve content and internal links; escaping links are never chmoded', { skip: process.platform !== 'linux' }, async () => {
  const working = await fs.mkdtemp(path.join(os.tmpdir(), 'mewu-appdir-permissions-'));
  await fs.chmod(working, 0o700);
  const root = path.join(working, 'AppDir'), outside = path.join(working, 'outside');
  await fs.mkdir(root, { mode: 0o700 });
  await fs.mkdir(path.join(root, 'schemas')); await fs.chmod(path.join(root, 'schemas'), 0o777);
  await fs.writeFile(path.join(root, 'schemas/gschemas.compiled'), 'controlled schema fixture');
  await fs.chmod(path.join(root, 'schemas/gschemas.compiled'), 0o666);
  await fs.writeFile(path.join(root, 'helper'), 'controlled inert executable fixture'); await fs.chmod(path.join(root, 'helper'), 0o6777);
  await fs.symlink('schemas/gschemas.compiled', path.join(root, 'internal'));
  await fs.writeFile(outside, 'outside must retain content and mode'); await fs.chmod(outside, 0o666);
  try {
    const result = await normalizeAppDir(root, { requireResources: false });
    assert.equal(result.changed, 3);
    assert.equal((await fs.stat(path.join(root, 'helper'))).mode & 0o7777, 0o755);
    assert.equal((await fs.stat(path.join(root, 'schemas/gschemas.compiled'))).mode & 0o7777, 0o644);
    assert.equal(await fs.readlink(path.join(root, 'internal')), 'schemas/gschemas.compiled');
    await fs.symlink('../outside', path.join(root, 'escape'));
    await assert.rejects(normalizeAppDir(root, { requireResources: false }), /escapes/);
    assert.equal((await fs.stat(outside)).mode & 0o7777, 0o666);
    assert.equal(await fs.readFile(outside, 'utf8'), 'outside must retain content and mode');
  } finally { await fs.rm(working, { recursive: true, force: true }); }
});

test('native missing resources reject before repack', { skip: process.platform !== 'linux' }, async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'mewu-appdir-missing-'));
  try { await assert.rejects(inspectAppDir(root), /resource missing/); }
  finally { await fs.rm(root, { recursive: true, force: true }); }
});

test('native graphics frontend cannot silently bypass the host driver policy', { skip: process.platform !== 'linux' }, async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'mewu-appdir-old-gles-'));
  try {
    await fs.mkdir(path.join(root, 'usr/lib'), { recursive: true });
    await fs.writeFile(path.join(root, 'usr/lib/libGLESv2.so.2'), 'controlled inert old frontend fixture');
    await assert.rejects(inspectAppDir(root), /Host graphics provider/);
    // Raw rollback copies are compared byte-for-byte without pretending the
    // pre-staging input already satisfies the final runtime resource contract.
    const original = await inspectAppDir(root, { requireResources: false });
    assert.ok(original.entries.some(entry => entry.relative === 'usr/lib/libGLESv2.so.2'));
  } finally { await fs.rm(root, { recursive: true, force: true }); }
});

test('unsupported platforms do not change output or launch tools', { skip: process.platform === 'linux' && process.arch === 'x64' }, async () => {
  await assert.rejects(rebuildAppImage('nonexistent-AppDir', 'nonexistent-output', 'nonexistent-tool'), /native Linux/);
});

test('host nghttp2 cannot shadow the system OCR curl provider', { skip: process.platform !== 'linux' }, async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'mewu-appdir-host-network-'));
  try {
    await fs.mkdir(path.join(root, 'usr/lib'), { recursive: true });
    for (const name of ['libnghttp2.so.14', 'libnghttp2.so.14.17.1']) {
      const fixture = path.join(root, 'usr/lib', name);
      await fs.writeFile(fixture, 'controlled inert old network provider fixture');
      await assert.rejects(inspectAppDir(root), /Host network provider/);
      const original = await inspectAppDir(root, { requireResources: false });
      assert.ok(original.entries.some(entry => entry.relative === `usr/lib/${name}`));
      await fs.unlink(fixture);
    }
  } finally { await fs.rm(root, { recursive: true, force: true }); }
});
