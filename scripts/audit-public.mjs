// SPDX-License-Identifier: MPL-2.0
// Audit tracked source before publishing. Findings contain paths, never secrets.
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
const index = process.argv.includes('--index');
const paths = execFileSync('git', index ? ['ls-files', '--cached', '-z'] : ['ls-tree', '-r', '--name-only', '-z', 'HEAD'], { encoding: 'utf8' }).split('\0').filter(Boolean);
const problems = [];
const forbidden = /(?:^|\/)(?:\.private|node_modules|target|dist|out|artifacts|\.vs|__pycache__)(?:\/|$)|(?:^|\/)(?:agents?\.md|claude\.md|gemini\.md|\.env(?:\..*)?)$|\.(?:key|pem|pfx|p12|sqlite(?:3)?|db|log|dmp|pdb|tmp|user)$/i;
const secrets = [/(?:gh[pousr]_[A-Za-z0-9_]{30,}|github_pat_[A-Za-z0-9_]{35,})/g,
  /-----BEGIN (?:RSA |EC |OPENSSH |ENCRYPTED )?PRIVATE KEY-----/g,
  /untrusted comment:\s*(?:minisign )?(?:encrypted )?secret key/gi,
  /C:[\\/]Users[\\/](?!Public[\\/]|Default[\\/]|test[\\/]|fixture[\\/]|example[\\/])[^\\/\s]+[\\/]/gi];
for (const path of paths) {
  // git ls-files can retain a removed path until its deletion is staged.
  if (index && !fs.existsSync(path)) { problems.push(`${path}: unstaged deletion`); continue; }
  if (forbidden.test(path)) { problems.push(`${path}: private/generated file`); continue; }
  const bytes = execFileSync('git', ['show', `${index ? ':' : 'HEAD:'}${path}`], { maxBuffer: 32 * 1024 * 1024 });
  if (bytes.subarray(0, 8192).includes(0)) continue;
  const text = bytes.toString('utf8');
  if (secrets.some(pattern => { pattern.lastIndex = 0; return pattern.test(text); })) problems.push(`${path}: private content`);
}
if (problems.length) { console.error(problems.join('\n')); process.exitCode = 1; }
else console.log(`Public source audit passed: ${paths.length} files`);
