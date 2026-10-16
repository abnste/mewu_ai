// SPDX-License-Identifier: MPL-2.0
// GitHub's tag endpoint does not expose draft releases. Authenticated release
// listings do; keep that fallback so interrupted uploads can resume one draft.
function read(gh, endpoint, ...flags) {
  try { return JSON.parse(gh('api', endpoint, ...flags)); }
  catch (error) { if (/\(HTTP 404\)/.test(String(error.stderr))) return null; throw error; }
}

export function findRelease(gh, repository, tag) {
  const published = read(gh, `repos/${repository}/releases/tags/${tag}`);
  if (published) return published;
  const pages = read(gh, `repos/${repository}/releases?per_page=100`, '--paginate', '--slurp');
  if (!Array.isArray(pages) || pages.some(page => !Array.isArray(page))) throw Error('Invalid release listing');
  const matches = pages.flat().filter(release => release.tag_name === tag);
  if (matches.length > 1) throw Error('Multiple releases have the same tag; resolve drafts before publishing');
  return matches[0] ?? null;
}

export function verifyReleaseTarget(gh, repository, release, tag, commit, preview) {
  if (!release || release.tag_name !== tag || typeof release.draft !== 'boolean' || release.prerelease !== preview) throw Error('Existing release identity differs');
  // A draft may not have created its Git ref yet. Only accept its exact source
  // SHA, never a moving branch name; if a ref exists it must agree as well.
  if (release.draft && release.target_commitish !== commit) throw Error('Draft release source differs');
  const ref = read(gh, `repos/${repository}/git/ref/tags/${tag}`);
  if (!ref) {
    if (!release.draft) throw Error('Published release tag is missing');
    return;
  }
  if (ref.object?.type !== 'commit' || ref.object.sha !== commit) throw Error('Existing release tag source differs');
}

// The original 0.7.4 updater reads /latest rather than the Tauri JSON feed.
// Confirm explicit stable selection, even if an archived numeric bridge tag
// sorts above the real product version under System.Version.
export async function verifyStableLatest(gh, repository, expected, { pause = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds)) } = {}) {
  if (!expected || !Number.isSafeInteger(expected.id) || expected.id < 1 || expected.draft !== false || expected.prerelease !== false) throw Error('Published stable release required for latest verification');
  // Publishing can become visible before the /latest read replica catches up.
  // Re-read only: four checks, 500 ms apart, with no second publish or upload.
  for (let attempt = 0; attempt < 4; attempt++) {
    const latest = read(gh, `repos/${repository}/releases/latest`);
    if (latest?.id === expected.id && latest.tag_name === expected.tag_name && latest.draft === false && latest.prerelease === false) return latest;
    if (attempt < 3) await pause(500);
  }
  throw Error('GitHub latest does not identify the published stable release');
}
