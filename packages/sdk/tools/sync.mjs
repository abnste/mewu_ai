// SPDX-License-Identifier: MPL-2.0
import { readFile, writeFile } from 'node:fs/promises';

const source = new URL('../../../crates/mewu-extension/src/sdk.js', import.meta.url);
const target = new URL('../index.mjs', import.meta.url);
const args = process.argv.slice(2);
if (args.length !== 1 || !['--check', '--write'].includes(args[0])) {
  throw new Error('Usage: node tools/sync.mjs --check|--write');
}
const embedded = await readFile(source);
if (embedded.subarray(0, 3).equals(Buffer.from([0xef, 0xbb, 0xbf]))) {
  throw new Error('Embedded SDK must be UTF-8 without BOM');
}
if (args[0] === '--write') await writeFile(target, embedded);
const published = await readFile(target);
if (!embedded.equals(published)) throw new Error('Published SDK differs from embedded runtime; run npm run sync');
const appearanceSource = new URL('../../../crates/mewu-extension/src/appearance-contract.json', import.meta.url);
const appearanceTarget = new URL('../appearance.json', import.meta.url);
const appearance = await readFile(appearanceSource);
if(args[0]==='--write') await writeFile(appearanceTarget,appearance);
if(!appearance.equals(await readFile(appearanceTarget))) throw new Error('Published appearance contract differs from host; run npm run sync');
const colors=JSON.parse(appearance).colors;
const defaults='/* SPDX-License-Identifier: MPL-2.0 */\n/* Generated from the public appearance contract. */\n:root{'+Object.values(colors).flatMap(token=>token.css.map(css=>css+':'+token.light+';')).join('')+'}\n';
const defaultsTarget=new URL('../../../apps/desktop/src/design-tokens.css',import.meta.url);
if(args[0]==='--write')await writeFile(defaultsTarget,defaults);
if((await readFile(defaultsTarget,'utf8'))!==defaults)throw new Error('Host design tokens differ from the appearance contract');
process.stdout.write('Embedded and published SDK bytes match.\n');
