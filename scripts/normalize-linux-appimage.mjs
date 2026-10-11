// SPDX-License-Identifier: MPL-2.0
// Build-only: normalize a newly generated AppDir, repack, verify, then stage.
// Never called by application startup; never executes the candidate application.
import fs from 'node:fs/promises';
import { createReadStream } from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const MAX_ENTRIES = 4096, MAX_DEPTH = 16, MAX_FILE = 512 * 1024 * 1024, MAX_TOTAL = 2 * 1024 * 1024 * 1024;
const REQUIRED = [
  ['usr/bin/MewuAI', 'file'], ['usr/lib/gtk-3.0', 'dir'],
  ['usr/lib/gtk-3.0/3.0.0/immodules.cache', 'file'],
  ['usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache', 'file'],
  ['usr/lib/girepository-1.0', 'dir'], ['usr/lib/gio/modules', 'dir'],
  ['usr/share/glib-2.0/schemas', 'dir'], ['usr/lib/gstreamer-1.0', 'dir'],
  ['usr/lib/gstreamer1.0/gstreamer-1.0/gst-plugin-scanner', 'file'],
  ['usr/lib/gstreamer1.0/gstreamer-1.0/gst-ptp-helper', 'file'],
];
function fail(message) { throw Error(message); }
function contained(root, value) {
  const relative = path.relative(root, value);
  return relative === '' || (!path.isAbsolute(relative) && relative !== '..' && !relative.startsWith(`..${path.sep}`));
}
function same(a, b) { return a.dev === b.dev && a.ino === b.ino && a.size === b.size && a.mtimeMs === b.mtimeMs; }
async function fingerprint(file) {
  const before = await fs.lstat(file);
  if (!before.isFile() || before.size > MAX_FILE) fail('Expected bounded regular file');
  const digest = createHash('sha256');
  for await (const chunk of createReadStream(file)) digest.update(chunk);
  if (!same(before, await fs.lstat(file))) fail('File changed during hash');
  return { bytes: before.size, sha256: digest.digest('hex') };
}
export async function inspectAppDir(input, { normalized = false, requireResources = true } = {}) {
  const root = await fs.realpath(input), rootStat = await fs.lstat(input);
  if (!rootStat.isDirectory() || root !== path.resolve(input)) fail('AppDir root must be a real directory');
  const entries = [], euid = process.geteuid(), visited = new Set();
  let totalBytes = 0;
  async function visit(file, depth) {
    if (entries.length >= MAX_ENTRIES || depth > MAX_DEPTH || Buffer.byteLength(file) > 4096) fail('AppDir bounds exceeded');
    const metadata = await fs.lstat(file), relative = path.relative(root, file).split(path.sep).join('/');
    if (metadata.dev !== rootStat.dev || (metadata.uid !== 0 && metadata.uid !== euid)) fail('Untrusted AppDir device or owner');
    const mode = metadata.mode & 0o7777;
    if (metadata.isSymbolicLink()) {
      const target = await fs.readlink(file), resolved = await fs.realpath(file);
      if (path.isAbsolute(target) || !contained(root, resolved)) fail('AppDir symlink escapes');
      entries.push({ relative, kind: 'symlink', target });
      return; // Never traverse or chmod symlinks.
    }
    if (normalized && (mode & 0o6022)) fail('Writable or privileged AppDir entry');
    if (metadata.isDirectory()) {
      entries.push({ relative, kind: 'dir', mode });
      const identity = `${metadata.dev}:${metadata.ino}`;
      if (visited.has(identity)) fail('Unexpected duplicate directory identity');
      visited.add(identity);
      for (const name of (await fs.readdir(file)).sort()) await visit(path.join(file, name), depth + 1);
    } else if (metadata.isFile()) {
      const hash = await fingerprint(file);
      totalBytes += hash.bytes;
      if (totalBytes > MAX_TOTAL) fail('AppDir byte limit exceeded');
      entries.push({ relative, kind: 'file', mode, ...hash });
    } else fail('Special AppDir entry refused');
  }
  await visit(root, 0);
  if (requireResources) for (const [relative, kind] of REQUIRED) {
    const entry = entries.find(e => e.relative === relative);
    if (!entry || entry.kind !== kind) fail('Required package resource missing');
    if ((relative === 'usr/bin/MewuAI' || relative.endsWith('gst-plugin-scanner') || relative.endsWith('gst-ptp-helper')) && !(entry.mode & 0o111)) fail('Required executable missing permission');
  }
  return { root, entries, totalBytes };
}
function equalContent(left, right, compareModes) {
  const content = entries => entries.map(({ relative, kind, target, bytes, sha256, mode }) => ({ relative, kind, target, bytes, sha256, ...(compareModes && kind !== 'symlink' ? { mode } : {}) })).sort((a, b) => a.relative.localeCompare(b.relative));
  if (JSON.stringify(content(left.entries)) !== JSON.stringify(content(right.entries))) fail('Repack changed package content or permissions');
}
export async function normalizeAppDir(input, options = {}) {
  if (process.platform !== 'linux') fail('Permission normalization requires native Linux');
  const before = await inspectAppDir(input, options);
  let changed = 0;
  for (const entry of before.entries) if (entry.kind !== 'symlink') {
    const mode = entry.mode & ~0o6022;
    if (mode !== entry.mode) { await fs.chmod(path.join(before.root, entry.relative), mode); changed++; }
  }
  const after = await inspectAppDir(input, { ...options, normalized: true });
  equalContent(before, after, false);
  return { before, after, changed };
}

// ELF64 little-endian headers, not an hsqsh-string search. Section/program
// file ranges locate the appended filesystem; SHT_NOBITS has no file payload.
// https://www.man7.org/linux/man-pages/man5/elf.5.html
export function squashfsOffset(bytes) {
  if (bytes.length < 64 || bytes.length > MAX_FILE || bytes.subarray(0, 4).toString('hex') !== '7f454c46'
      || bytes[4] !== 2 || bytes[5] !== 1 || bytes[6] !== 1 || bytes.subarray(8, 11).toString('hex') !== '414902'
      || ![2, 3].includes(bytes.readUInt16LE(16)) || bytes.readUInt16LE(18) !== 62
      || bytes.readUInt32LE(20) !== 1 || bytes.readUInt16LE(52) !== 64) fail('Expected bounded x86_64 ELF64 type-2 AppImage');
  const number = offset => {
    const value = bytes.readBigUInt64LE(offset);
    if (value > BigInt(bytes.length)) fail('ELF file offset exceeds file');
    return Number(value);
  };
  let end = bytes.readUInt16LE(52);
  for (const [offset, stride, count, minimum, payloadOffset, payloadSize, sections] of [
    [number(32), bytes.readUInt16LE(54), bytes.readUInt16LE(56), 56, 8, 32, false],
    [number(40), bytes.readUInt16LE(58), bytes.readUInt16LE(60), 64, 24, 32, true],
  ]) {
    if (!count || count > 512 || stride < minimum || offset + stride * count > bytes.length) fail('Invalid bounded ELF header table');
    end = Math.max(end, offset + stride * count);
    for (let index = 0; index < count; index++) {
      const entry = offset + index * stride;
      if (sections && bytes.readUInt32LE(entry + 4) === 8) continue;
      const start = number(entry + payloadOffset), length = number(entry + payloadSize);
      if (start + length > bytes.length) fail('ELF payload exceeds file');
      end = Math.max(end, start + length);
    }
  }
  if (bytes.subarray(end, end + 4).toString('hex') !== '68737173') fail('No SquashFS at ELF file boundary');
  return end;
}

export async function rebuildAppImage(appdir, output, plugin) {
  if (process.platform !== 'linux' || process.arch !== 'x64') fail('Repacking requires native Linux x86_64');
  const repository = await fs.realpath(process.cwd()), bundle = await fs.realpath(path.join(repository, 'target/release/bundle/appimage'));
  if (!contained(repository, bundle) || bundle !== path.join(repository, 'target/release/bundle/appimage')) fail('Build output must stay inside this workspace');
  appdir = path.resolve(appdir); output = path.resolve(output); plugin = path.resolve(plugin);
  if (appdir !== path.join(bundle, 'MewuAI.AppDir') || path.dirname(output) !== bundle
      || !/^MewuAI_[0-9][A-Za-z0-9.+-]*_amd64\.AppImage$/.test(path.basename(output))) fail('Only newly built workspace AppDir/output allowed');
  const cache = await fs.realpath(path.join(os.homedir(), '.cache/tauri'));
  if (plugin !== path.join(cache, 'linuxdeploy-plugin-appimage.AppImage')) fail('Expected this build cached output plugin');
  const originalArtifact = await fingerprint(output), tool = await fingerprint(plugin);
  const sourceCommit = execFileSync('git', ['-C', repository, 'rev-parse', 'HEAD'], { encoding: 'utf8', timeout: 10_000, maxBuffer: 4096 }).trim();
  if (!/^[a-f0-9]{40}$/.test(sourceCommit) || (process.env.MEWU_BUILD_COMMIT && process.env.MEWU_BUILD_COMMIT !== sourceCommit)) fail('Build source identity mismatch');
  const working = await fs.mkdtemp(path.join(bundle, '.permission-repack-'));
  await fs.chmod(working, 0o700);
  // Copy the already used build plugin into a private tool file. Never ship it.
  const privatePlugin = path.join(working, 'linuxdeploy-plugin-appimage.AppImage');
  await fs.copyFile(plugin, privatePlugin); await fs.chmod(privatePlugin, 0o700);
  if ((await fingerprint(privatePlugin)).sha256 !== tool.sha256) fail('Cached plugin changed during copy');
  const normalized = await normalizeAppDir(appdir);
  const candidate = path.join(working, path.basename(output));
  const env = { PATH: '/usr/local/bin:/usr/bin:/bin', LANG: 'C.UTF-8', ARCH: 'x86_64', OUTPUT: candidate, LDAI_OUTPUT: candidate, APPIMAGE_EXTRACT_AND_RUN: '1', TMPDIR: working };
  const run = (file, args) => execFileSync(file, args, { cwd: working, env, timeout: 180_000, maxBuffer: 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
  run(privatePlugin, ['--appimage-extract-and-run', '--appdir', appdir]);
  equalContent(normalized.after, await inspectAppDir(appdir, { normalized: true }), true);
  const candidateHash = await fingerprint(candidate), bytes = await fs.readFile(candidate), offset = squashfsOffset(bytes);
  // System reader only: it reads the new SquashFS, never invokes Mewu/AppRun.
  const extracted = path.join(working, 'verified-tree');
  run('/usr/bin/unsquashfs', ['-no-progress', '-d', extracted, '-o', String(offset), candidate]);
  const verified = await inspectAppDir(extracted, { normalized: true });
  equalContent(normalized.after, verified, true);
  if (JSON.stringify(await fingerprint(output)) !== JSON.stringify(originalArtifact)) fail('Original build artifact changed concurrently');
  if (JSON.stringify(await fingerprint(candidate)) !== JSON.stringify(candidateHash)) fail('Repacked artifact changed during verification');
  const receipt = { schemaVersion: 1, kind: 'build_permission_normalization', sourceCommit, changedEntries: normalized.changed, totalEntries: verified.entries.length, totalContentBytes: verified.totalBytes, originalArtifact, outputArtifact: candidateHash, cachedPlugin: tool, squashfsOffset: offset, contentAndModesVerified: true, candidateExecuted: false, guiVerified: false, installerVerified: false };
  // Every verification precedes replacement of this build's unpublished output.
  await fs.writeFile(path.join(bundle, 'permission-normalization.json'), `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o600 });
  await fs.rename(candidate, output);
  // Only our uniquely created build scratch directory, never the AppDir/cache.
  // Cleanup cannot turn an already verified/staged replacement into an apparent
  // pre-replacement failure. Deferred scratch is diagnostic, never a GUI pass.
  try { await fs.rm(working, { recursive: true, force: true }); }
  catch { console.error('Private build scratch cleanup deferred'); }
  return receipt;
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [appdir, output, plugin, ...extra] = process.argv.slice(2);
  if (!appdir || !output || !plugin || extra.length) fail('Usage: node scripts/normalize-linux-appimage.mjs AppDir output cached-plugin');
  console.log(JSON.stringify(await rebuildAppImage(appdir, output, plugin)));
}
