// SPDX-License-Identifier: MPL-2.0
// Run from any directory after npm ci: node apps/desktop/scripts/generate-third-party-notices.mjs
// Requires Cargo and ripgrep on PATH. Reads locked package metadata and package license files;
// does not infer licenses from source headers or claim a binary distribution audit.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../../..');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const cargoLock = readFileSync(resolve(root, 'Cargo.lock'));
const npmLockBytes = readFileSync(resolve(root, 'package-lock.json'));
const npmLock = JSON.parse(npmLockBytes);
const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--locked', '--format-version', '1'], {
  cwd: root, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024,
}));
const windowsMetadata = JSON.parse(execFileSync('cargo', ['metadata', '--locked', '--format-version', '1',
  '--filter-platform', 'x86_64-pc-windows-msvc'], { cwd: root, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 }));
const windowsNodes = new Map(windowsMetadata.resolve.nodes.map(node => [node.id, node]));
const windowsPackages = new Set();
const pendingWindows = [...windowsMetadata.workspace_members];
while (pendingWindows.length) {
  const id = pendingWindows.pop();
  if (windowsPackages.has(id)) continue;
  windowsPackages.add(id);
  for (const dependency of windowsNodes.get(id)?.deps ?? []) {
    if (dependency.dep_kinds.some(kind => kind.kind !== 'dev')) pendingWindows.push(dependency.pkg);
  }
}
const workspace = new Set(metadata.workspace_members);
const directRust = new Map();
for (const node of metadata.resolve.nodes.filter(node => workspace.has(node.id))) {
  for (const dependency of node.deps) {
    if (workspace.has(dependency.pkg)) continue;
    const uses = directRust.get(dependency.pkg) ?? new Set();
    for (const kind of dependency.dep_kinds) uses.add(kind.kind ?? 'runtime');
    directRust.set(dependency.pkg, uses);
  }
}
const appPackage = JSON.parse(readFileSync(resolve(root, 'apps/desktop/package.json'), 'utf8'));
const directNode = new Map([
  ...Object.keys(appPackage.dependencies ?? {}).map(name => [name, 'runtime']),
  ...Object.keys(appPackage.devDependencies ?? {}).map(name => [name, 'build/dev']),
]);
const packages = metadata.packages.filter(pkg => !workspace.has(pkg.id)).map(pkg => ({
  ecosystem: 'Cargo', name: pkg.name, version: pkg.version, license: pkg.license ?? 'UNDECLARED',
  scope: directRust.has(pkg.id) ? `direct (${[...directRust.get(pkg.id)].sort().join(', ')})` : 'transitive',
  root: dirname(pkg.manifest_path), licenseFile: pkg.license_file, windows: windowsPackages.has(pkg.id),
  source: pkg.source?.startsWith('git+') ? pkg.source.slice(4) : `https://crates.io/crates/${pkg.name}/${pkg.version}`,
  gitSource: pkg.source,
}));
for (const [location, pkg] of Object.entries(npmLock.packages)) {
  if (pkg.link || !location.includes('node_modules/')) continue;
  const name = pkg.name ?? location.split('node_modules/').at(-1);
  const packageRoot = resolve(root, location);
  let installed = false;
  try { installed = JSON.parse(readFileSync(resolve(packageRoot, 'package.json'), 'utf8')).version === pkg.version; }
  catch { /* Optional packages for other operating systems need not be installed. */ }
  packages.push({ ecosystem: 'npm', name, version: pkg.version, license: pkg.license ?? 'UNDECLARED',
    scope: directNode.has(name) ? `direct (${directNode.get(name)})` : `transitive${pkg.dev ? ' (build/dev)' : ''}`,
    root: installed ? packageRoot : undefined,
    source: pkg.resolved ?? `https://www.npmjs.com/package/${name}/v/${pkg.version}`,
  });
}
packages.sort((a, b) => `${a.ecosystem}:${a.name}@${a.version}`.localeCompare(`${b.ecosystem}:${b.name}@${b.version}`, 'en'));

const searchRoots = new Set(packages.filter(pkg => pkg.ecosystem === 'Cargo').map(pkg => dirname(pkg.root)));
for (const candidate of ['node_modules', 'apps/desktop/node_modules']) {
  const directory = resolve(root, candidate);
  if (existsSync(directory)) searchRoots.add(directory);
}
const discovered = execFileSync('rg', ['--files', '--hidden', '--no-ignore',
  '--iglob', '*license*', '--iglob', '*licence*', '--iglob', '*copying*', '--iglob', '*copyright*',
  '--iglob', '*notice*', '--iglob', '*unlicense*', '--glob', '!**/.git/**', ...searchRoots],
  { encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 }).trim().split(/\r?\n/).filter(Boolean);
const legalName = /^(?:licen[cs]e|copying|copyright|notice|unlicense)(?:[.\-_].*)?$/i;
const codeExtension = /\.(?:rs|js|jsx|ts|tsx|mjs|cjs|c|h|cpp|py|json|map|wasm|toml)$/i;
const isInside = (directory, path) => {
  const local = relative(directory, path);
  return local !== '' && local !== '..' && !local.startsWith(`..${sep}`) && !isAbsolute(local);
};
const texts = new Map();
const skipped = [];
const gplTextCandidates = [];
for (const pkg of packages) {
  pkg.evidence = [];
  if (!pkg.root) continue;
  const candidates = discovered.filter(path => {
    if (!isInside(pkg.root, path)) return false;
    const local = relative(pkg.root, path);
    if (local.split(sep).includes('node_modules')) return false;
    const name = local.split(sep).at(-1);
    return legalName.test(name) && !codeExtension.test(name);
  });
  if (pkg.licenseFile) {
    const declared = resolve(pkg.root, pkg.licenseFile);
    if (isInside(pkg.root, declared) && existsSync(declared)) candidates.push(declared);
  }
  for (const path of [...new Set(candidates)].sort()) {
    const sourcePath = relative(pkg.root, path).split(sep).join('/');
    // r-efi offers three alternatives. This inventory selects MIT, not LGPL.
    if (pkg.name === 'r-efi' && /(?:LGPL|APACHE)/i.test(sourcePath)) continue;
    try {
      if (statSync(path).size > 2 * 1024 * 1024) throw new Error('license file exceeds 2 MiB');
      const text = new TextDecoder('utf-8', { fatal: true }).decode(readFileSync(path)).replace(/\r\n/g, '\n').trimEnd();
      if (!text || text.includes('\0')) throw new Error('not UTF-8 text');
      const digest = hash(text);
      if (!texts.has(digest)) texts.set(digest, { id: `L${String(texts.size + 1).padStart(3, '0')}`, text, uses: [] });
      const evidence = texts.get(digest);
      evidence.uses.push(`${pkg.ecosystem} ${pkg.name}@${pkg.version} — ${sourcePath}`);
      pkg.evidence.push(`${sourcePath} [${evidence.id}]`);
      if (/GNU (?:AFFERO |LESSER )?GENERAL PUBLIC LICENSE/i.test(text)) {
        gplTextCandidates.push({ package: `${pkg.name}@${pkg.version}`, path: sourcePath, declared: pkg.license });
      }
    } catch (error) { skipped.push(`${pkg.name}@${pkg.version}: ${sourcePath} (${error.message})`); }
  }
}

// The exact upstream 0.2.3 tree and crate omit LICENSE/NOTICE files, but both
// Cargo.toml and README.md explicitly offer MIT OR Apache-2.0. Select Apache-2.0
// and supply its standard text; never fabricate an upstream MIT copyright line.
const eventSource = packages.find(pkg => pkg.ecosystem === 'Cargo' && pkg.name === 'eventsource-stream' && pkg.version === '0.2.3');
if (eventSource) {
  const vcs = JSON.parse(readFileSync(resolve(eventSource.root, '.cargo_vcs_info.json'), 'utf8'));
  if (vcs.git.sha1 !== '3d46f1c758f9ee4681e9da0427556d24c53f9c01' || eventSource.license !== 'MIT OR Apache-2.0') {
    throw new Error('eventsource-stream provenance changed; review the selected license before regenerating');
  }
  const standardBytes = readFileSync(resolve(root, 'apps/desktop/licenses/Apache-2.0.txt'));
  if (hash(standardBytes) !== 'cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30') {
    throw new Error('The reviewed Apache-2.0 standard text changed');
  }
  const text = standardBytes.toString('utf8').replace(/\r\n/g, '\n').trimEnd();
  const digest = hash(text);
  if (!texts.has(digest)) texts.set(digest, { id: `L${String(texts.size + 1).padStart(3, '0')}`, text, uses: [] });
  const evidence = texts.get(digest);
  evidence.uses.push('Cargo eventsource-stream@0.2.3 — selected Apache-2.0 standard text from https://www.apache.org/licenses/LICENSE-2.0.txt; this is not represented as an upstream LICENSE file');
  eventSource.evidence.push(`selected Apache-2.0 standard text [${evidence.id}]; exact upstream declaration in dedicated section`);
}

// Some published crate archives omit their repository's root legal text.
// Supplement only from the exact published VCS revision, with pinned bytes.
// KaTeX's JavaScript MIT notice does not cover its separately licensed fonts.
// These copyright/RFN records were parsed from the locked TTF name tables;
// pinned bytes make regeneration fail rather than silently reuse stale notices.
const katexFonts = JSON.parse(readFileSync(resolve(root, 'apps/desktop/licenses/KaTeX-fonts.json'), 'utf8'));
const katex = packages.find(pkg => pkg.ecosystem === 'npm' && pkg.name === 'katex');
if (katex) {
  if (!katex.root || katex.version !== katexFonts.version ||
      npmLock.packages['node_modules/katex']?.integrity !== katexFonts.integrity) {
    throw new Error('KaTeX font package changed; review binary font notices');
  }
  const standard = readFileSync(resolve(root, 'apps/desktop/licenses', katexFonts.standard.file));
  if (hash(standard) !== katexFonts.standard.sha256) throw new Error('Reviewed OFL standard text changed');
  const notices = new Map();
  for (const font of katexFonts.fonts) {
    if (!/^[A-Za-z0-9_-]+\.ttf$/.test(font.file) ||
        hash(readFileSync(resolve(katex.root, 'dist/fonts', font.file))) !== font.sha256) {
      throw new Error('KaTeX font binary changed; review copyright and reserved names');
    }
    const notice = font.names.filter(name => name.id === 0 || name.id === 13).map(name => name.text).join('\n\n');
    if (!notice.includes('SIL Open Font License, Version 1.1')) throw new Error('Missing font license declaration');
    const files = notices.get(notice) ?? [];
    files.push(font.file);
    notices.set(notice, files);
  }
  const text = [...notices].map(([notice, files]) => `${files.join(', ')}\n\n${notice}`).join('\n\n') +
    '\n\nOfficial OFL 1.1 standard text (template headers are not additional font copyright claims):\n\n' +
    standard.toString('utf8').replace(/\r\n/g, '\n').trimEnd();
  const digest = hash(text);
  if (!texts.has(digest)) texts.set(digest, { id: `L${String(texts.size + 1).padStart(3, '0')}`, text, uses: [] });
  const evidence = texts.get(digest);
  evidence.uses.push(`npm katex@${katex.version} — font copyright/RFN declarations from pinned TTF name tables; OFL 1.1 standard text from ${katexFonts.standard.source}`);
  katex.evidence.push(`separate font OFL-1.1 notice [${evidence.id}]; code MIT notice remains separate`);
}

const supplements = JSON.parse(readFileSync(resolve(root, 'apps/desktop/licenses/upstream-supplements.json'), 'utf8'));
for (const supplement of supplements) {
  const pkg = packages.find(pkg => pkg.ecosystem === 'Cargo' && pkg.name === supplement.name && pkg.version === supplement.version);
  if (!pkg) continue;
  const revision = supplement.git ? pkg.gitSource?.split('#').at(-1)
    : JSON.parse(readFileSync(resolve(pkg.root, '.cargo_vcs_info.json'), 'utf8')).git.sha1;
  if (revision !== supplement.commit) throw new Error(`${pkg.name} legal-source revision changed`);
  const directory = resolve(root, 'apps/desktop/licenses');
  const sourceFile = resolve(directory, supplement.file);
  if (!isInside(directory, sourceFile)) throw new Error('Legal supplement is outside the license directory');
  const bytes = readFileSync(sourceFile);
  if (hash(bytes) !== supplement.sha256) throw new Error(`${pkg.name} upstream legal text changed`);
  const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes).replace(/\r\n/g, '\n').trimEnd();
  const digest = hash(text);
  if (!texts.has(digest)) texts.set(digest, { id: `L${String(texts.size + 1).padStart(3, '0')}`, text, uses: [] });
  const evidence = texts.get(digest);
  evidence.uses.push(`Cargo ${pkg.name}@${pkg.version} — exact published commit ${supplement.commit}, ${supplement.source}; SHA-256 ${supplement.sha256}`);
  pkg.evidence.push(`${supplement.standard ? 'declared license standard text' : 'verified repository LICENSE supplement'} [${evidence.id}]`);
}

// The native formula renderer embeds the pinned RaTeX font payload as well as
// frontend KaTeX fonts. Retain the SDK's full attribution and OFL separately.
for (const name of ['LICENSE', 'THIRD_PARTY_NOTICES.txt', 'KaTeX-fonts-NOTICE.txt', 'SIL-OFL-1.1.txt']) {
  const bytes = readFileSync(resolve(root, 'apps/desktop/src-tauri/resources/formula/licenses', name));
  const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes).replace(/\r\n/g, '\n').trimEnd();
  const digest = hash(text);
  if (!texts.has(digest)) texts.set(digest, { id: `L${String(texts.size + 1).padStart(3, '0')}`, text, uses: [] });
  texts.get(digest).uses.push(`Bundled native RaTeX formula SDK and KaTeX font resources — ${name}; upstream revision 776c1d37bafa3bf445a0ab9377c55fe77f7a0133`);
}
for (const supplement of JSON.parse(readFileSync(resolve(root, 'apps/desktop/licenses/binary-supplements.json'), 'utf8'))) {
  const directory = resolve(root, 'apps/desktop/licenses');
  const filename = resolve(directory, supplement.file);
  if (!isInside(directory, filename)) throw new Error('Invalid binary notice path');
  const bytes = readFileSync(filename);
  if (hash(bytes) !== supplement.sha256) throw new Error('Reviewed binary license changed');
  const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes).replace(/\r\n/g, '\n').trimEnd();
  const digest = hash(text);
  if (!texts.has(digest)) texts.set(digest, { id: `L${String(texts.size + 1).padStart(3, '0')}`, text, uses: [] });
  texts.get(digest).uses.push(`${supplement.component} — ${supplement.source}, archive member ${supplement.member}; SHA-256 ${supplement.sha256}`);
}

const direct = packages.filter(pkg => pkg.scope.startsWith('direct'));
const transitive = packages.filter(pkg => !pkg.scope.startsWith('direct'));
const noTexts = packages.filter(pkg => pkg.evidence.length === 0);
const missingWindows = noTexts.filter(pkg => pkg.ecosystem === 'Cargo' && pkg.windows);
if (missingWindows.length) throw new Error(`Missing Windows license texts: ${missingWindows.map(pkg => pkg.name).join(', ')}`);
const plain = value => String(value).replaceAll('|', '\\|').replaceAll('\n', ' ');
const rows = entries => entries.map(pkg => `| ${pkg.ecosystem} | ${plain(pkg.name)} | ${pkg.version} | ${plain(pkg.license)} | ${pkg.scope} | ${pkg.evidence.length ? plain(pkg.evidence.join('; ')) : 'METADATA ONLY — license/copyright text not collected'} |`).join('\n');
const sections = [
  '# Mewu 1.0 desktop — third-party notices',
  '',
  `Generated from the locked dependency graph on ${new Date().toISOString().slice(0, 10)}.`,
  '',
  'This notice belongs to the Rust/Tauri/Solid desktop application. Legacy WPF dependency notices remain with the legacy releases; those binaries are not shipped in this release.',
  '',
  '## Scope and evidence',
  '',
  `- Cargo.lock SHA-256: ${hash(cargoLock)}`,
  `- package-lock.json SHA-256: ${hash(npmLockBytes)}`,
  `- ${packages.filter(pkg => pkg.ecosystem === 'Cargo').length} external Cargo packages and ${packages.filter(pkg => pkg.ecosystem === 'npm').length} external npm package entries. Workspace-owned packages and workspace symlinks are excluded.`,
  `- ${packages.filter(pkg => pkg.ecosystem === 'Cargo' && pkg.windows).length} external Cargo packages are reachable in the default-feature x86_64-pc-windows-msvc metadata graph, including build dependencies. The larger all-platform inventory is not a list of Windows-linked libraries.`,
  '- Cargo declarations come from cargo metadata --locked --format-version 1 and the registry packages resolved by Cargo.lock. npm declarations come from package-lock.json; installed package versions are checked before reading their files.',
  '- The inventory includes runtime, build-time, and optional packages for other operating systems. Inclusion does not mean that every listed package is linked into the Windows executable or that every platform is implemented.',
  '- Each L-number below identifies a complete collected UTF-8 LICENSE, COPYING, COPYRIGHT, or NOTICE text, or an explicitly identified standard-license supplement. Identical texts are stored once with all source-package references. Relative paths are inside the relevant third-party package; no private development records are included.',
  '- Output formatting normalizes line endings and removes trailing whitespace on each line; all license wording is retained. Empty quoted lines are rendered as > without a trailing space.',
  `- ${packages.length - noTexts.length} package entries have collected legal text; ${noTexts.length} have declarations only. Missing text is explicitly marked below. ${texts.size} distinct texts are included.`,
  '- Every package in the Windows non-dev dependency graph has complete collected legal text. Declaration-only entries below concern optional, development or other-platform packages; they are not evidence of a license review for a future platform release. Bundled mathematical font notices are retained separately below. System WebView2 and Windows media/graphics runtimes are supplied by Microsoft, not relicensed by Mewu.',
  '- r-efi 5.3.0 and 6.0.0 declare MIT OR Apache-2.0 OR LGPL-2.1-or-later. MIT is the selected alternative for this inventory; unselected LGPL/Apache text is not used as the license of Mewu. Other OR expressions retain their upstream alternatives; AND components apply together.',
  '- aws-lc-sys 0.45.0 includes an upstream statement expressly selecting the BSD-3-Clause alternative for Jitter Entropy, rather than GPL-2.0. That complete statement is retained below; its mention of GPL is not a selection of GPL for this dependency.',
  '- The all-platform libdbus-sys source package includes a vendored D-Bus COPYING file offering AFL-2.1 or GPL-2.0 and identifying some standalone GPL-only tools. libdbus-sys is absent from the current Windows dependency graph. Its source-package notice is retained as evidence, not as a claim those tools are built, copied, or distributed by Mewu. Any future Linux or vendored-D-Bus distribution must separately confirm the applicable alternative and exclude/review standalone tools.',
  '- MPL-2.0 text discusses GPL as a possible secondary license; those references do not change the selected MPL license. The TypeScript upstream NOTICE contains a conditional LGPL debugging clause, not a declaration that TypeScript itself is LGPL.',
  '- The Creative Commons declaration on caniuse-lite concerns its browser-support dataset, used by build tooling. It is not a license applied to the Mewu application.',
  '- rmcp 3.5.1 declares Apache-2.0 in Cargo metadata. Its exact published revision LICENSE also describes the MIT-to-Apache transition and retains MIT for contributions not yet relicensed. That complete upstream notice is included without simplifying it to a single repository-wide license; its documentation clause concerns documentation, not the Mewu application.',
  '- rxing 0.9.3 is an Apache-2.0 Rust port of ZXing with portions derived from zxing-cpp. Its published README retains the original ZXing authors and zxing-cpp contributors as copyright holders; those rights remain with them. rxing-one-d-proc-derive 0.9.1 also declares Apache-2.0. Their complete published license texts are included below. encoding_rs additionally requires the BSD-3-Clause WHATWG notice, included alongside its Apache/MIT alternatives.',
  '- Mewu-authored code is licensed separately under MPL-2.0. Third-party rights and notices remain with their respective holders. This file does not relicense any dependency.',
  '',
  'Reproduce with Cargo, Node.js, and ripgrep on PATH, after npm ci: node apps/desktop/scripts/generate-third-party-notices.mjs',
  '',
  '## eventsource-stream 0.2.3 — selected Apache-2.0 alternative', '',
  '- Package author recorded by the exact upstream Cargo.toml: Julian Popescu. This identifies the author and is not a fabricated copyright notice.',
  '- Crate archive checksum: 74fef4569247a5f429d9156b9d0a2599914385dd189c539334c625d8099d90ab.',
  '- Published .cargo_vcs_info.json pins commit 3d46f1c758f9ee4681e9da0427556d24c53f9c01.',
  '- Exact upstream declaration: https://github.com/jpopesculian/eventsource-stream/blob/3d46f1c758f9ee4681e9da0427556d24c53f9c01/Cargo.toml (license = "MIT OR Apache-2.0"); the same declaration appears in README.md at that commit.',
  '- The published crate and the complete official tree at https://api.github.com/repos/jpopesculian/eventsource-stream/git/trees/3d46f1c758f9ee4681e9da0427556d24c53f9c01?recursive=1 contain no LICENSE or NOTICE file. This absence was checked rather than treated as a download failure.',
  '- For this exact version Mewu selects Apache-2.0. Its complete standard text is included below and in licenses/Apache-2.0.txt, retrieved from https://www.apache.org/licenses/LICENSE-2.0.txt. SHA-256: cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30.',
  '- No upstream LICENSE/NOTICE file or copyright year has been invented, and no unverified template attribution has been substituted for upstream evidence.',
  '',
  '## Direct dependencies', '',
  '| Ecosystem | Package | Locked version | Declared license | Role | Collected source files |',
  '| --- | --- | --- | --- | --- | --- |', rows(direct), '',
  '## Transitive dependencies', '',
  '| Ecosystem | Package | Locked version | Declared license | Role | Collected source files |',
  '| --- | --- | --- | --- | --- | --- |', rows(transitive), '',
  '## Package-source locators', '',
  ...packages.map(pkg => `- ${pkg.ecosystem} ${pkg.name}@${pkg.version}: ${pkg.source}`), '',
  '## Collected upstream legal texts', '',
];
for (const entry of texts.values()) {
  sections.push(`### ${entry.id}`, '', 'Source packages:', ...entry.uses.map(use => `- ${use}`), '', '----- BEGIN UPSTREAM TEXT -----', entry.text, '----- END UPSTREAM TEXT -----', '');
}
if (skipped.length) sections.push('## Files not collected', '', ...skipped.map(item => `- ${item}`), '');
const destination = resolve(root, 'apps/desktop/THIRD-PARTY-NOTICES');
writeFileSync(destination, sections.join('\n').split('\n').map(line => line.trimEnd()).join('\n'), 'utf8');
process.stdout.write(`${JSON.stringify({ packages: packages.length, direct: direct.length, texts: texts.size,
  metadataOnly: noTexts.map(pkg => `${pkg.ecosystem} ${pkg.name}@${pkg.version}`), skipped,
  gplTextCandidates, bytes: statSync(destination).size }, null, 2)}\n`);
