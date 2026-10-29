// SPDX-License-Identifier: MPL-2.0
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path';

/** A path patch is the project's source, never the unchanged registry archive. */
export function cargoLegalSource(projectRoot, pkg) {
  if (typeof pkg.source === 'string') return {
    source: pkg.source.startsWith('git+') ? pkg.source.slice(4) : `https://crates.io/crates/${encodeURIComponent(pkg.name)}/${encodeURIComponent(pkg.version)}`,
    localSource: false,
  };
  const local = relative(resolve(projectRoot), dirname(resolve(pkg.manifest_path)));
  if (local === '..' || local.startsWith(`..${sep}`) || isAbsolute(local)) throw new Error('Local Cargo legal source is outside the project');
  return { source: `Mewu source tree: ${local.split(sep).join('/') || '.'}`, localSource: true };
}
