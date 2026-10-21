// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { inspectPortableCli, inspectNativeCli, inspectNsisCli, portableFiles } from './cli-package.mjs';

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const privateRoot = path.join(repository, '.private/remake-review/cli-package-20261020');
fs.mkdirSync(privateRoot, { recursive: true });
function fixture(run) {
  const directory = fs.mkdtempSync(path.join(privateRoot, 'fixture-'));
  try { return run(directory); }
  finally { assert.equal(path.dirname(fs.realpathSync(directory)), fs.realpathSync(privateRoot)); fs.rmSync(directory, { recursive: true }); }
}
function put(root, relative, contents = 'inert package fixture') {
  const file = path.join(root, relative); fs.mkdirSync(path.dirname(file), { recursive: true }); fs.writeFileSync(file, contents); fs.chmodSync(file, 0o700); return file;
}

test('portable contains exactly the five public files and both executable fingerprints', () => fixture(root => {
  for (const name of portableFiles) put(root, name);
  const result = inspectPortableCli(root);
  assert.deepEqual(result.map(value => value.path), portableFiles);
  fs.unlinkSync(path.join(root, 'mewu.exe'));
  assert.throws(() => inspectPortableCli(root), /exact five/);
  put(root, 'mewu.exe'); put(root, 'private.txt');
  assert.throws(() => inspectPortableCli(root), /exact five/);
}));
for (const [platform, required] of [
  ['windows-x86_64', ['MewuAI.exe', 'mewu.exe']],
  ['darwin-aarch64', ['Contents/MacOS/MewuAI', 'Contents/MacOS/mewu', 'Contents/Resources/cli/integrate-unix.sh']],
  ['linux-x86_64', ['usr/bin/MewuAI', 'usr/bin/mewu', 'usr/lib/MewuAI/cli/integrate-unix.sh']],
]) test(`${platform} native layout requires the real console sibling and excludes development validators`, () => fixture(root => {
  for (const name of required) put(root, name);
  assert.equal(inspectNativeCli(root, platform).length, required.length);
  const consoleFile = required.find(value => /(?:^|\/)mewu(?:\.exe)?$/.test(value));
  fs.unlinkSync(path.join(root, consoleFile));
  assert.throws(() => inspectNativeCli(root, platform), /ENOENT/);
  put(root, consoleFile);
  put(root, path.join(path.dirname(consoleFile), platform === 'windows-x86_64' ? 'mewu-plugin-validator.exe' : 'mewu-plugin-validator'));
  assert.throws(() => inspectNativeCli(root, platform), /Development validator/);
}));

test('actual Windows PowerShell path planner preserves long values, ownership and user spelling without touching HKCU', { skip: process.platform !== 'win32' }, () => fixture(root => {
  const owner = (directory, addedEntry = directory, createdPath = false) => ({ schemaVersion: 1, managedBy: 'dev.mewu.remake.cli.v1', installDirectory: directory, addedEntry, createdPath });
  const directory = 'C:\\Users\\Test Person\\MewuAI';
  const cases = [
    { Action: 'Install', CurrentPath: '', PathExists: false, Directory: directory, Ownership: null },
    { Action: 'Install', CurrentPath: `C:\\Tools;"${directory.toLowerCase()}\\"`, PathExists: true, Directory: directory, Ownership: null },
    { Action: 'Install', CurrentPath: `C:\\Tools;${directory}`, PathExists: true, Directory: directory, Ownership: owner(directory) },
    { Action: 'Uninstall', CurrentPath: `C:\\Tools;${directory};C:\\Other`, PathExists: true, Directory: directory, Ownership: owner(directory) },
    { Action: 'Uninstall', CurrentPath: `C:\\Tools;${directory.toLowerCase()}`, PathExists: true, Directory: directory, Ownership: owner(directory) },
    { Action: 'Uninstall', CurrentPath: directory, PathExists: true, Directory: directory, Ownership: owner(directory, directory, true) },
    { Action: 'Uninstall', CurrentPath: `C:\\Tools;${directory}`, PathExists: true, Directory: 'D:\\Other MewuAI', Ownership: owner(directory) },
    { Action: 'Install', CurrentPath: `C:\\Tools;;${directory}`, PathExists: true, Directory: 'D:\\New MewuAI', Ownership: owner(directory) },
    { Action: 'Install', CurrentPath: 'C:\\Tools;', PathExists: true, Directory: directory, Ownership: null },
    { Action: 'Install', CurrentPath: Array.from({ length: 700 }, (_, index) => `C:\\LongTool${index}`).join(';'), PathExists: true, Directory: directory, Ownership: null },
    { Action: 'Install', CurrentPath: 'C:\\Tools', PathExists: true, Directory: 'C:\\Bad;Path', Ownership: null },
    { Action: 'Uninstall', CurrentPath: 'C:\\Windows', PathExists: true, Directory: directory, Ownership: owner(directory, 'C:\\Windows') },
  ];
  const casesFile = put(root, 'cases.json', JSON.stringify(cases));
  const helper = path.join(repository, 'apps/desktop/src-tauri/resources/cli/integrate-windows.ps1');
  const quote = value => `'${value.replaceAll("'", "''")}'`;
  const runner = put(root, 'plan.ps1', `. ${quote(helper)} -LibraryOnly\n$cases = Get-Content -LiteralPath ${quote(casesFile)} -Raw | ConvertFrom-Json\n$result = @(foreach ($case in $cases) { try { Get-MewuPathPlan -Action $case.Action -CurrentPath $case.CurrentPath -PathExists $case.PathExists -Directory $case.Directory -Ownership $case.Ownership } catch { [PSCustomObject]@{ error = [string]$_.Exception.Message } } })\nConvertTo-Json -InputObject $result -Depth 12 -Compress\n`);
  const result = JSON.parse(execFileSync('powershell.exe', ['-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', runner], { encoding: 'utf8' }));
  assert.equal(result[0].path, directory); assert.equal(result[0].ownership.createdPath, true);
  assert.equal(result[1].write, false); assert.equal(result[1].ownership.addedEntry, '');
  assert.equal(result[2].write, false); assert.equal(result[2].ownership.addedEntry, directory);
  assert.equal(result[3].path, 'C:\\Tools;C:\\Other');
  assert.equal(result[4].write, false); assert.equal(result[4].path, cases[4].CurrentPath);
  assert.equal(result[5].deletePath, true);
  assert.equal(result[6].write, false); assert.equal(result[6].clearOwnership, false);
  assert.equal(result[7].path, 'C:\\Tools;;D:\\New MewuAI');
  assert.equal(result[8].path, `C:\\Tools;;${directory}`);
  assert.equal(result[9].path, `${cases[9].CurrentPath};${directory}`); assert.ok(result[9].path.length > 1024);
  assert.equal(result[10].error, 'CLI_PATH_INVALID_DIRECTORY');
  assert.equal(result[11].error, 'CLI_PATH_INVALID_OWNERSHIP');
}));

test('platform configs package the noninteractive Unix integrator without publishing another asset', () => {
  for (const platform of ['macos', 'linux']) {
    const config = JSON.parse(fs.readFileSync(path.join(repository, `apps/desktop/src-tauri/tauri.${platform}.conf.json`)));
    assert.equal(config.bundle.resources['resources/cli/integrate-unix.sh'], 'cli/integrate-unix.sh');
  }
  const hooks = fs.readFileSync(path.join(repository, 'apps/desktop/src-tauri/installer-hooks.nsh'), 'utf8');
  assert.ok(hooks.includes('Call MewuInstallCliPath') && hooks.includes('Call un.MewuUninstallCliPath'));
  assert.ok(hooks.includes('File /oname=mewu-cli-path.ps1'));
  for (const source of ['resources/cli/integrate-windows.ps1', 'resources/cli/integrate-unix.sh']) {
    const bytes = fs.readFileSync(path.join(repository, 'apps/desktop/src-tauri', source));
    assert.notEqual(bytes.subarray(0, 3).toString('hex'), 'efbbbf');
  }
});

const bash = process.platform === 'win32' ? 'C:/Program Files/Git/bin/bash.exe' : '/bin/sh';
test('Windows installer registry integration survives notification failure on both PowerShell architectures', { skip: process.platform !== 'win32' }, () => fixture(root => {
  put(root, 'MewuAI.exe'); put(root, 'mewu.exe');
  const helper = path.join(repository, 'apps/desktop/src-tauri/resources/cli/integrate-windows.ps1');
  for (const architecture of ['System32', 'SysWOW64']) {
    const powershell = path.join(process.env.SystemRoot, architecture, 'WindowsPowerShell/v1.0/powershell.exe');
    const result = JSON.parse(execFileSync(powershell, ['-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', path.join(repository, 'scripts/cli-windows-registry.fixture.ps1'), '-Helper', helper, '-Directory', root], { encoding: 'utf8', timeout: 30000 }));
    assert.equal(result.success, true);
    assert.equal(result.notificationFailurePreservesInstall, true);
  }
}));
const shellAvailable = fs.existsSync(bash);
function shellPath(file) {
  if (process.platform !== 'win32') return file;
  // A scoped TEMP makes MSYS alias this fixture tree as /tmp. Use the stable
  // drive mount so the production AppImage guard still rejects real /tmp.
  const absolute = path.resolve(file).replaceAll('\\', '/');
  assert.match(absolute, /^[A-Za-z]:\//);
  return `/${absolute[0].toLowerCase()}${absolute.slice(2)}`;
}
function integrate(args, env = {}, commandPath) {
  const script = shellPath(path.join(repository, 'apps/desktop/src-tauri/resources/cli/integrate-unix.sh'));
  const environment = { ...process.env };
  for (const [key, value] of Object.entries(env)) {
    if (process.platform === 'win32') for (const oldKey of Object.keys(environment)) if (oldKey.toLowerCase() === key.toLowerCase()) delete environment[oldKey];
    if (value !== undefined) environment[key] = value;
  }
  // Git Bash's launcher prepends /usr/bin. Only the private uname fixture needs
  // a tool path installed after startup; no production test flag or eval.
  const unsetShell = Object.hasOwn(env, 'SHELL') && env.SHELL === undefined;
  const shellArgs = commandPath ? ['-c', 'PATH=$1; export PATH; shift; exec /bin/sh "$@"', 'fixture', commandPath, script, ...args]
    : unsetShell ? ['-c', 'unset SHELL; script=$1; shift; . "$script"', 'fixture', script, ...args] : [script, ...args];
  return spawnSync(bash, shellArgs, { encoding: 'utf8', env: environment });
}
const shellSkip = !shellAvailable || process.platform !== 'win32' && process.getuid?.() === 0;
test('actual Unix shell integrator preserves foreign commands and forwards literal arguments through an owned Mac launcher', { skip: shellSkip }, () => fixture(root => {
  const app = path.join(root, "Mewu's Application.app"), bin = path.join(root, 'user bin');
  put(app, 'Contents/MacOS/MewuAI', '#!/bin/sh\nexit 0\n');
  put(app, 'Contents/MacOS/mewu', '#!/bin/sh\nprintf "%s\\n" "$@"\n');
  const original = put(bin, 'mewu', 'an unrelated user command\n');
  const args = ['--macos-app', shellPath(app), '--bin-dir', shellPath(bin)];
  const refused = integrate(args); assert.notEqual(refused.status, 0); assert.match(refused.stderr, /EXISTING_COMMAND_PRESERVED/);
  assert.equal(fs.readFileSync(original, 'utf8'), 'an unrelated user command\n');
  fs.unlinkSync(original);
  const installed = integrate(args); assert.equal(installed.status, 0, installed.stderr);
  assert.equal(JSON.parse(installed.stdout).changed, true);
  const repeated = integrate(args); assert.equal(repeated.status, 0, repeated.stderr); assert.equal(JSON.parse(repeated.stdout).changed, false);
  const forwarded = spawnSync(bash, [shellPath(path.join(bin, 'mewu')), 'hello world', "quote's literal", 'a;b'], { encoding: 'utf8' });
  assert.equal(forwarded.status, 0, forwarded.stderr); assert.equal(forwarded.stdout, "hello world\nquote's literal\na;b\n");
  const removed = integrate(['--uninstall', '--bin-dir', shellPath(bin)]); assert.equal(removed.status, 0, removed.stderr); assert.equal(fs.existsSync(original), false);
}));

test('actual Unix shell integrator pins the stable AppImage file and refuses a FUSE path', { skip: shellSkip }, () => fixture(root => {
  const image = put(root, "Mewu's stable.AppImage", '#!/bin/sh\nprintf "%s\\n" "$@"\n');
  const bin = path.join(root, 'appimage bin');
  const installed = integrate(['--appimage', shellPath(image), '--bin-dir', shellPath(bin)]);
  assert.equal(installed.status, 0, installed.stderr);
  const shim = fs.readFileSync(path.join(bin, 'mewu'), 'utf8');
  assert.ok(!shim.includes('APPDIR') && !shim.includes('.mount_'));
  const forwarded = spawnSync(bash, [shellPath(path.join(bin, 'mewu')), 'app.status', 'literal value'], { encoding: 'utf8' });
  assert.equal(forwarded.status, 0, forwarded.stderr); assert.equal(forwarded.stdout, '--cli\napp.status\nliteral value\n');
  const fuse = put(root, '.mount_fixture/usr/bin/MewuAI', '#!/bin/sh\nexit 0\n');
  const rejected = integrate(['--appimage', shellPath(fuse), '--bin-dir', shellPath(bin)]);
  assert.notEqual(rejected.status, 0); assert.match(rejected.stderr, /PATH_NOT_PERSISTENT/);
  assert.equal(fs.readFileSync(path.join(bin, 'mewu'), 'utf8'), shim);
}));

function shellFixture(root, shell = 'bash', extraEnv = {}) {
  const home = path.join(root, 'isolated home'); fs.mkdirSync(home);
  const bin = path.join(home, "bin's commands");
  const image = put(root, 'stable.AppImage', '#!/bin/sh\nprintf "%s\\n" "$@"\n');
  const env = { HOME: shellPath(home), SHELL: `/bin/${shell}`, ZDOTDIR: '', ...extraEnv };
  // Match the native platform's documented Bash entry point. The dedicated
  // Darwin priority test below independently exercises existing login files.
  const profileName = process.platform === 'darwin' ? '.bash_profile' : '.bashrc';
  return { home, bin, env, profileName, install: ['--appimage', shellPath(image), '--bin-dir', shellPath(bin), '--configure-shell'], uninstall: ['--uninstall', '--bin-dir', shellPath(bin)] };
}
test('shell PATH configuration is opt-in and reports an unsupported shell honestly', { skip: shellSkip }, () => fixture(root => {
  const f = shellFixture(root, 'fish');
  const result = integrate(f.install, f.env); assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout), { success: true, changed: true, pathConfigured: false, shellSetupRequired: true, newShellRequired: true });
  assert.equal(fs.existsSync(path.join(f.home, '.bashrc')), false); assert.equal(fs.existsSync(path.join(f.home, '.zshrc')), false);
  f.env.SHELL = undefined;
  const unset = integrate(f.install, f.env); assert.equal(unset.status, 0, unset.stderr); assert.equal(JSON.parse(unset.stdout).shellSetupRequired, true);
  const removed = integrate(f.uninstall, f.env); assert.equal(removed.status, 0, removed.stderr); assert.equal(JSON.parse(removed.stdout).shellSetupRequired, true);
}));
test('bash managed PATH block preserves original bytes, is idempotent, works in a new interactive shell, and uninstalls exactly', { skip: shellSkip }, () => fixture(root => {
  const f = shellFixture(root);
  const original = Buffer.concat([Buffer.from('# 原有 UTF-8 + raw '), Buffer.from([0xff]), Buffer.from('\r\nexport ORIGINAL_RC_VALUE=kept')]);
  const profile = put(f.home, f.profileName, original);
  const result = integrate(f.install, f.env); assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout), { success: true, changed: true, pathConfigured: false, shellSetupRequired: false, newShellRequired: true });
  const installed = fs.readFileSync(profile); assert.deepEqual(installed.subarray(0, original.length), original);
  assert.equal(installed.toString().split('# >>> mewu CLI PATH v1 >>>').length, 2);
  const repeated = integrate(f.install, f.env); assert.equal(repeated.status, 0, repeated.stderr); assert.equal(JSON.parse(repeated.stdout).changed, false);
  assert.deepEqual(fs.readFileSync(profile), installed);
  // Invoking macOS Bash as sh ignores --rcfile, per Bash's startup contract.
  const interactiveBash = process.platform === 'win32' ? bash : '/bin/bash';
  const next = spawnSync(interactiveBash, ['--noprofile', '--rcfile', shellPath(profile), '-ic', 'command -v mewu; printf "%s\\n" "$ORIGINAL_RC_VALUE"'], { encoding: 'utf8', env: { ...process.env, ...f.env } });
  assert.equal(next.status, 0, next.stderr); assert.equal(next.stdout, `${shellPath(path.join(f.bin, 'mewu'))}\nkept\n`);
  const removed = integrate(f.uninstall, f.env); assert.equal(removed.status, 0, removed.stderr); assert.equal(JSON.parse(removed.stdout).newShellRequired, true);
  assert.deepEqual(fs.readFileSync(profile), original); assert.equal(fs.existsSync(path.join(f.bin, 'mewu')), false);
  // Preserve text added after our block too, including its absent trailing LF.
  const again = integrate(f.install, f.env); assert.equal(again.status, 0, again.stderr);
  const suffix = Buffer.from('# user added after installation, no final LF'); fs.appendFileSync(profile, suffix);
  const removedAgain = integrate(f.uninstall, f.env); assert.equal(removedAgain.status, 0, removedAgain.stderr);
  assert.deepEqual(fs.readFileSync(profile), Buffer.concat([original, suffix]));
}));
test('zsh PATH block uses ZDOTDIR and preserves user edits to its own block on uninstall', { skip: shellSkip }, () => fixture(root => {
  const f = shellFixture(root, 'zsh'); const zdot = path.join(f.home, 'zsh settings'); fs.mkdirSync(zdot); f.env.ZDOTDIR = shellPath(zdot);
  const profile = put(zdot, '.zshrc', '# existing zsh\n');
  const result = integrate(f.install, f.env); assert.equal(result.status, 0, result.stderr); assert.equal(JSON.parse(result.stdout).shellSetupRequired, false);
  assert.equal(fs.existsSync(path.join(f.home, '.zshrc')), false);
  const edited = fs.readFileSync(profile, 'utf8').replace('export PATH', 'export PATH # user edit'); fs.writeFileSync(profile, edited);
  const repeated = integrate(f.install, f.env); assert.equal(repeated.status, 0, repeated.stderr); assert.equal(JSON.parse(repeated.stdout).shellSetupRequired, true);
  const removed = integrate(f.uninstall, f.env); assert.equal(removed.status, 0, removed.stderr); assert.equal(JSON.parse(removed.stdout).shellSetupRequired, true);
  assert.equal(fs.readFileSync(profile, 'utf8'), edited);
}));
test('shell profile links, nonordinary files and oversized contents are preserved', { skip: shellSkip }, () => fixture(root => {
  const f = shellFixture(root); const profile = path.join(f.home, f.profileName);
  fs.mkdirSync(profile);
  let result = integrate(f.install, f.env); assert.equal(result.status, 0, result.stderr); assert.equal(JSON.parse(result.stdout).shellSetupRequired, true); assert.equal(fs.statSync(profile).isDirectory(), true);
  fs.rmdirSync(profile); fs.writeFileSync(profile, Buffer.alloc(1024 * 1024 + 1, 120));
  result = integrate(f.install, f.env); assert.equal(result.status, 0, result.stderr); assert.equal(JSON.parse(result.stdout).shellSetupRequired, true); assert.equal(fs.statSync(profile).size, 1024 * 1024 + 1);
  fs.unlinkSync(profile);
  // ln -s on MSYS can copy instead of linking without Developer Mode. A native
  // Unix run exercises the actual symlink refusal, rather than claiming a copy.
  if (process.platform !== 'win32') {
    const other = put(f.home, 'other-profile', '# leave intact\n'); fs.symlinkSync(other, profile);
    result = integrate(f.install, f.env); assert.equal(result.status, 0, result.stderr); assert.equal(JSON.parse(result.stdout).shellSetupRequired, true);
    assert.equal(fs.lstatSync(profile).isSymbolicLink(), true); assert.equal(fs.readFileSync(other, 'utf8'), '# leave intact\n');
  }
}));
test('macOS bash uses the existing login startup file in documented priority order', { skip: shellSkip }, () => fixture(root => {
  const fakeTools = path.join(root, 'fixture tools'); put(fakeTools, 'uname', '#!/bin/sh\nprintf "Darwin\\n"\n');
  const commandPath = `${shellPath(fakeTools)}:/usr/bin:/bin`;
  const f = shellFixture(root, 'bash');
  const login = put(f.home, '.bash_login', '# existing login\n'), fallback = put(f.home, '.profile', '# existing fallback\n');
  const result = integrate(f.install, f.env, commandPath); assert.equal(result.status, 0, result.stderr); assert.equal(JSON.parse(result.stdout).shellSetupRequired, false);
  assert.equal(fs.existsSync(path.join(f.home, '.bash_profile')), false); assert.equal(fs.existsSync(path.join(f.home, '.bashrc')), false);
  assert.ok(fs.readFileSync(login, 'utf8').includes('# >>> mewu CLI PATH v1 >>>')); assert.equal(fs.readFileSync(fallback, 'utf8'), '# existing fallback\n');
  const removed = integrate(f.uninstall, f.env, commandPath); assert.equal(removed.status, 0, removed.stderr); assert.equal(fs.readFileSync(login, 'utf8'), '# existing login\n');
}));
test('shell block conflicts and profile locks never overwrite user configuration', { skip: shellSkip }, () => fixture(root => {
  const f = shellFixture(root); const profile = put(f.home, f.profileName, '# >>> mewu CLI PATH v1 >>>\n# unrelated or incomplete\n'); const original = fs.readFileSync(profile);
  let result = integrate(f.install, f.env); assert.equal(result.status, 0, result.stderr); assert.equal(JSON.parse(result.stdout).shellSetupRequired, true); assert.deepEqual(fs.readFileSync(profile), original);
  fs.writeFileSync(profile, '# clean original\n'); fs.mkdirSync(`${profile}.mewu-cli.lock`);
  result = integrate(f.install, f.env); assert.equal(result.status, 0, result.stderr); assert.equal(JSON.parse(result.stdout).shellSetupRequired, true); assert.equal(fs.readFileSync(profile, 'utf8'), '# clean original\n');
  fs.rmdirSync(`${profile}.mewu-cli.lock`);
}));
test('opt-in shell setup creates a private startup file and uses the original GUI PATH only for diagnostics', { skip: shellSkip }, () => fixture(root => {
  const f = shellFixture(root); const profile = path.join(f.home, f.profileName);
  const result = integrate(f.install.filter(value => value !== '--configure-shell'), f.env); assert.equal(result.status, 0, result.stderr);
  assert.equal(fs.existsSync(profile), false); assert.equal(JSON.parse(result.stdout).shellSetupRequired, true);
  const configured = integrate(f.install, { ...f.env, MEWU_CLI_ORIGINAL_PATH: `${shellPath(f.bin)}:/other/original/gui/path` });
  assert.equal(configured.status, 0, configured.stderr);
  assert.deepEqual(JSON.parse(configured.stdout), { success: true, changed: true, pathConfigured: true, shellSetupRequired: false, newShellRequired: false });
  assert.ok(fs.readFileSync(profile, 'utf8').includes('# >>> mewu CLI PATH v1 >>>'));
  if (process.platform !== 'win32') assert.equal(fs.statSync(profile).mode & 0o777, 0o600);
  const removed = integrate(f.uninstall, f.env); assert.equal(removed.status, 0, removed.stderr); assert.equal(fs.readFileSync(profile, 'utf8'), '');
}));
test('a profile edit observed while taking the bounded snapshot is preserved', { skip: shellSkip }, () => fixture(root => {
  const f = shellFixture(root); const profile = put(f.home, f.profileName, '# user original\n');
  const tools = path.join(root, 'fixture tools');
  put(tools, 'head', '#!/bin/sh\n/usr/bin/head "$@"\nif [ "$1" = -c ] && [ "$2" = 1048577 ]; then printf "# concurrent user edit\\n" >> "$3"; fi\n');
  const result = integrate(f.install, f.env, `${shellPath(tools)}:/usr/bin:/bin`); assert.equal(result.status, 0, result.stderr);
  assert.equal(JSON.parse(result.stdout).shellSetupRequired, true); assert.equal(fs.readFileSync(profile, 'utf8'), '# user original\n# concurrent user edit\n');
  assert.equal(fs.readdirSync(f.home).some(name => name.startsWith('.mewu-cli-') || name.endsWith('.mewu-cli.lock')), false);
}));

test('removing a block at byte zero works with BSD head and preserves a following user suffix', { skip: shellSkip }, () => fixture(root => {
  const f = shellFixture(root); const profile = path.join(f.home, f.profileName);
  const tools = path.join(root, 'BSD tools');
  put(tools, 'head', '#!/bin/sh\nif [ "$1" = -c ] && [ "$2" = 0 ]; then printf "head: illegal byte count -- 0\\n" >&2; exit 1; fi\nexec /usr/bin/head "$@"\n');
  const commandPath = `${shellPath(tools)}:/usr/bin:/bin`;
  const installed = integrate(f.install, f.env, commandPath); assert.equal(installed.status, 0, installed.stderr);
  const suffix = Buffer.from('# preserve user suffix without final LF'); fs.appendFileSync(profile, suffix);
  const removed = integrate(f.uninstall, f.env, commandPath); assert.equal(removed.status, 0, removed.stderr);
  assert.equal(JSON.parse(removed.stdout).shellSetupRequired, false);
  assert.deepEqual(fs.readFileSync(profile), suffix);
}));

const nsis = path.join(process.env.LOCALAPPDATA || '', 'tauri/NSIS/makensis.exe');
test('actual NSIS compiler includes both owned PATH hooks and both ordinary binaries without running an installer', { skip: process.platform !== 'win32' || !fs.existsSync(nsis) }, () => fixture(root => {
  const installer = path.join(root, 'fixture-setup.exe');
  const host = put(root, 'inert-host.bin', 'inert host fixture, never execute');
  const cli = put(root, 'inert-cli.bin', 'inert console fixture, never execute');
  const hooks = path.join(repository, 'apps/desktop/src-tauri/installer-hooks.nsh');
  const source = put(root, 'fixture.nsi', `Unicode True\n!include "LogicLib.nsh"\n!define UNINSTKEY "Software\\MewuAI-Cli-Fixture-Never-Install"\n!include "${hooks}"\nName "Mewu CLI fixture: never install"\nOutFile "${installer}"\nInstallDir "$LOCALAPPDATA\\MewuAI-Cli-Fixture-Never-Install"\nRequestExecutionLevel user\nSection\n!insertmacro NSIS_HOOK_PREINSTALL\nSetOutPath "$INSTDIR"\nFile /oname=MewuAI.exe "${host}"\nFile /oname=mewu.exe "${cli}"\n!insertmacro NSIS_HOOK_POSTINSTALL\nWriteUninstaller "$INSTDIR\\uninstall.exe"\nSectionEnd\nSection "Uninstall"\n!insertmacro NSIS_HOOK_PREUNINSTALL\nDelete "$INSTDIR\\MewuAI.exe"\nDelete "$INSTDIR\\mewu.exe"\nSectionEnd\n`);
  const result = spawnSync(nsis, ['/V2', '/NOCD', source], { encoding: 'utf8', env: { ...process.env, TEMP: root, TMP: root } });
  assert.equal(result.status, 0, result.stdout + result.stderr); assert.ok(fs.statSync(installer).size > 0);
  const sevenZip = path.join(repository, '.private/remake-review/tools/7zip-26.04/unpacked/7z.exe');
  if (fs.existsSync(sevenZip)) {
    const verified = inspectNsisCli(installer, sevenZip, cli, repository);
    assert.equal(verified.installerExecuted, false);
    const extracted = path.join(root, 'extracted');
    execFileSync(sevenZip, ['x', '-y', `-o${extracted}`, installer], { stdio: 'pipe' });
    const all = fs.readdirSync(extracted, { recursive: true, withFileTypes: true }).filter(entry => entry.isFile()).map(entry => path.join(entry.parentPath, entry.name));
    const helper = all.find(file => path.basename(file) === 'mewu-cli-path.ps1');
    assert.ok(helper, 'compiled installer must embed the exact PATH helper');
    assert.deepEqual(fs.readFileSync(helper), fs.readFileSync(path.join(repository, 'apps/desktop/src-tauri/resources/cli/integrate-windows.ps1')));
    assert.deepEqual(fs.readFileSync(all.find(file => path.basename(file) === 'mewu.exe')), fs.readFileSync(cli));
  }
}));
