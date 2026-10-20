// SPDX-License-Identifier: MPL-2.0
// Build/staging checks only. Never executes a candidate or installs a command.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export const portableFiles = Object.freeze(['LICENSE', 'MewuAI.exe', 'SOURCE.md', 'THIRD-PARTY-NOTICES', 'mewu.exe']);
function file(root, relative, executable = false) {
  const candidate = path.join(root, relative), before = fs.lstatSync(candidate);
  if (!before.isFile() || before.isSymbolicLink() || before.size === 0 || before.size > 512 * 1024 * 1024) throw Error(`Required bounded CLI package file: ${relative}`);
  if (executable && process.platform !== 'win32' && !(before.mode & 0o111)) throw Error(`CLI package executable bit missing: ${relative}`);
  const bytes = fs.readFileSync(candidate), after = fs.lstatSync(candidate);
  if (before.dev !== after.dev || before.ino !== after.ino || before.size !== after.size || before.mtimeMs !== after.mtimeMs) throw Error('CLI package changed during inspection');
  return { path: relative, bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') };
}
export function inspectPortableCli(root) {
  const entries = fs.readdirSync(root).sort();
  if (JSON.stringify(entries) !== JSON.stringify([...portableFiles].sort())) throw Error('Portable payload must contain the exact five public files including mewu.exe');
  return portableFiles.map(relative => file(root, relative));
}
export function inspectNativeCli(root, platform) {
  const required = platform === 'windows-x86_64' ? ['MewuAI.exe', 'mewu.exe']
    : platform === 'darwin-aarch64' ? ['Contents/MacOS/MewuAI', 'Contents/MacOS/mewu', 'Contents/Resources/cli/integrate-unix.sh']
    : platform === 'linux-x86_64' ? ['usr/bin/MewuAI', 'usr/bin/mewu', 'usr/lib/MewuAI/cli/integrate-unix.sh'] : null;
  if (!required) throw Error('Unsupported CLI package platform');
  const binaryDirectory = path.join(root, platform === 'darwin-aarch64' ? 'Contents/MacOS' : platform === 'linux-x86_64' ? 'usr/bin' : '');
  if (fs.readdirSync(binaryDirectory).some(name => /^mewu-plugin-validator(?:\.exe)?$/i.test(name))) throw Error('Development validator must not enter a native release');
  return required.map(relative => file(root, relative, !relative.endsWith('.sh')));
}

export function inspectNsisCli(installer, extractor, consoleBinary, repository = process.cwd()) {
  file(path.dirname(installer), path.basename(installer));
  const parent = path.resolve(repository, '.private/remake-review/cli-package-extract');
  fs.mkdirSync(parent, { recursive: true });
  const temporary = fs.mkdtempSync(path.join(parent, 'candidate-'));
  try {
    execFileSync(extractor, ['x', '-y', `-o${temporary}`, installer], { stdio: 'pipe', maxBuffer: 2 * 1024 * 1024, timeout: 60_000 });
    const found = [];
    function walk(directory, depth = 0) {
      if (depth > 8 || found.length > 128) throw Error('NSIS CLI inspection bounds exceeded');
      for (const name of fs.readdirSync(directory)) {
        const candidate = path.join(directory, name), metadata = fs.lstatSync(candidate);
        if (metadata.isSymbolicLink()) throw Error('NSIS CLI package link refused');
        if (metadata.isDirectory()) walk(candidate, depth + 1);
        else if (metadata.isFile()) found.push(candidate);
        else throw Error('NSIS CLI package special file refused');
      }
    }
    walk(temporary);
    if (found.some(candidate => /^mewu-plugin-validator(?:\.exe)?$/i.test(path.basename(candidate)))) throw Error('Development validator must not enter an installer');
    const unique = name => { const values = found.filter(candidate => path.basename(candidate) === name); if (values.length !== 1) throw Error(`NSIS requires exactly one ${name}`); return values[0]; };
    const host = unique('MewuAI.exe'), consoleFile = unique('mewu.exe'), helper = unique('mewu-cli-path.ps1');
    if (path.dirname(host) !== path.dirname(consoleFile)) throw Error('NSIS console must be a real sibling of its host');
    const sourceHelper = path.join(repository, 'apps/desktop/src-tauri/resources/cli/integrate-windows.ps1');
    if (!fs.readFileSync(helper).equals(fs.readFileSync(sourceHelper))) throw Error('NSIS PATH helper differs from this source');
    const actual = file(path.dirname(consoleFile), path.basename(consoleFile)), expected = file(path.dirname(consoleBinary), path.basename(consoleBinary));
    if (actual.bytes !== expected.bytes || actual.sha256 !== expected.sha256) throw Error('NSIS console differs from the built console');
    return { console: actual, helper: file(path.dirname(helper), path.basename(helper)), installerExecuted: false };
  } finally {
    if (path.dirname(fs.realpathSync(temporary)) !== fs.realpathSync(parent)) throw Error('NSIS cleanup target changed');
    fs.rmSync(temporary, { recursive: true });
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const platform = process.argv[2], target = path.resolve(process.env.CARGO_TARGET_DIR || 'target');
  const version = JSON.parse(fs.readFileSync('apps/desktop/src-tauri/tauri.conf.json', 'utf8')).version;
  const result = platform === 'windows-x86_64'
    ? inspectNsisCli(path.resolve(process.argv[3] || path.join(target, `release/bundle/nsis/MewuAI_${version}_x64-setup.exe`)), process.argv[4] || '7z', path.join(target, 'release/mewu.exe'))
    : inspectNativeCli(path.resolve(process.argv[3] || path.join(target, platform === 'darwin-aarch64' ? 'release/bundle/macos/MewuAI.app' : 'release/bundle/appimage/MewuAI.AppDir')), platform);
  console.log(JSON.stringify({ platform, cliPackageVerified: true, result }));
}
