// SPDX-License-Identifier: MPL-2.0
// Fixed system-resource metadata only. This is not an OCR or GUI acceptance test.
import fs from 'node:fs';
import path from 'node:path';

if (process.platform !== 'linux') throw Error('Linux native diagnostic only');
if (!['x64', 'arm64'].includes(process.arch)) throw Error('Unsupported Linux architecture');
const libraryRoot = process.arch === 'arm64' ? '/usr/lib/aarch64-linux-gnu' : '/usr/lib/x86_64-linux-gnu';
const sources = [
  path.join(libraryRoot, 'libtesseract.so.5'), path.join(libraryRoot, 'libtesseract.so.4'),
  ...['/usr/share/tesseract-ocr/5/tessdata', '/usr/share/tesseract-ocr/4.00/tessdata', '/usr/share/tessdata']
    .flatMap(root => ['eng', 'chi_sim', 'chi_tra'].map(language => path.join(root, `${language}.traineddata`))),
];
function statRoute(target) {
  let result;
  try {
    const s = fs.lstatSync(target);
    result = {path: target, uid: s.uid, gid: s.gid, mode: (s.mode & 0o7777).toString(8),
      symlink: s.isSymbolicLink(), directory: s.isDirectory(), regular: s.isFile(), bytes: s.size};
  } catch (error) { return {path: target, error: error.code}; }
  try {
    const resolved = fs.realpathSync(target);
    if (resolved === '/usr' || resolved.startsWith('/usr/')) {
      result.resolved = resolved;
      const s = fs.statSync(target);
      result.resolvedIdentity = {uid: s.uid, gid: s.gid, mode: (s.mode & 0o7777).toString(8),
        dev: s.dev, ino: s.ino, bytes: s.size, mtimeMs: s.mtimeMs};
    } else { result.resolvedOutsideSystemTree = true; }
  } catch (error) { result.resolveError = error.code; }
  if (result.regular || result.symlink) {
    let fd;
    try { fd = fs.openSync(target, 'r'); result.readableByRunner = true; }
    catch (error) { result.readError = error.code; }
    finally { if (fd !== undefined) fs.closeSync(fd); }
  }
  return result;
}
const routes = new Set();
for (const source of sources) {
  for (let p = source; ; p = path.dirname(p)) {
    routes.add(p); if (p === '/') break;
  }
  try {
    const resolved = fs.realpathSync(source);
    if (resolved.startsWith('/usr/')) {
      for (let p = resolved; ; p = path.dirname(p)) { routes.add(p); if (p === '/') break; }
    }
  } catch {}
}
console.log(JSON.stringify({kind: 'native_linux_ocr_system_resource_diagnostic',
  uid: process.getuid(), gid: process.getgid(), arch: process.arch,
  loaderEnvironmentPresent: Object.fromEntries(['LD_LIBRARY_PATH', 'LD_PRELOAD', 'TESSDATA_PREFIX'].map(k => [k, Boolean(process.env[k])])),
  routes: [...routes].map(statRoute), productionPolicyChanged: false, nativeOcrPassed: false}, null, 2));
