// SPDX-License-Identifier: MPL-2.0
// GitHub's tag endpoint does not expose draft releases. Authenticated release
// listings do; keep that fallback so interrupted uploads can resume one draft.
import { compareReleaseVersions, publicationPolicy, releasePlan, releaseVersion, githubReleaseUpdates, validateReleaseNotes } from './release-contract.mjs';
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

// The modern publisher has no channel lookup or feed write. A transport can be
// injected for regression tests; production passes the same ordinary gh CLI.
export async function publishVersionRelease(gh, repository, { version, commit, artifacts, notes, expectedBody, verifyLocal, pause }) {
  const policy = publicationPolicy(version), tag = `v${version}`, plan = releasePlan(version);
  if (!policy.selfContained || !/^[a-f0-9]{40}$/.test(commit) || typeof verifyLocal !== 'function') throw Error('Modern source-bound release required');
  const direct = githubReleaseUpdates(version);
  const names = [...Object.values(plan).flatMap(p => p.files), plan['darwin-aarch64'].updater, ...(direct ? [] : ['latest.json'])];
  if (!Array.isArray(artifacts) || artifacts.length !== names.length || new Set(artifacts.map(a => a.name)).size !== names.length || names.some(n => !artifacts.some(a => a.name === n))) throw Error(`Exactly ${names.length} modern release assets required`);
  if (direct) validateReleaseNotes(expectedBody, version);
  const expected = new Map(artifacts.map(a => [a.name, a]));
  const lookup = () => findRelease(gh, repository, tag);
  const checkAsset = asset => {
    const wanted = expected.get(asset.name);
    if (!wanted || asset.size !== wanted.bytes || asset.digest !== `sha256:${wanted.sha256}`) throw Error('Remote release asset differs; never replace a versioned package');
  };
  const checkAssets = (release, complete = false) => {
    if (!Array.isArray(release?.assets) || new Set(release.assets.map(a => a.name)).size !== release.assets.length || (complete && release.assets.length !== names.length)) throw Error('Incomplete or duplicate remote release assets');
    for (const asset of release.assets) checkAsset(asset);
    if (direct && (typeof release.body !== 'string' || release.body.replace(/\r\n/g, '\n').trimEnd() !== expectedBody.replace(/\r\n/g, '\n').trimEnd())) throw Error('Remote release notes/signatures differ; never replace published metadata');
  };
  function checkAdvance() {
    if (policy.prerelease) return;
    const current = read(gh, `repos/${repository}/releases/latest`);
    if (!current) return;
    if (current.draft !== false || current.prerelease !== false) throw Error('Invalid current stable release');
    // These are historical old-client discovery versions, not product SemVer.
    if (/^v0\.\d+\.\d+$/.test(current.tag_name) || /^v1\.0\.0\.\d+$/.test(current.tag_name)) return;
    const currentVersion = releaseVersion(current.tag_name?.slice(1));
    if (compareReleaseVersions(version, currentVersion) < 0) throw Error('Refusing stable release downgrade');
  }
  verifyLocal(); checkAdvance();
  let current = lookup(), alreadyPublished = false;
  if (current) {
    verifyReleaseTarget(gh, repository, current, tag, commit, policy.prerelease);
    checkAssets(current);
    alreadyPublished = !current.draft;
    if (alreadyPublished && current.assets.length !== names.length) throw Error('Published release incomplete; never mutate published packages');
  } else {
    gh('release', 'create', tag, '--repo', repository, '--target', commit, '--draft', ...(policy.prerelease ? ['--prerelease'] : []), '--title', `MewuAI ${version} · 喵呜AI ${version}`, '--notes-file', notes);
    current = lookup(); verifyReleaseTarget(gh, repository, current, tag, commit, policy.prerelease);
    checkAssets(current);
  }
  if (!alreadyPublished) {
    for (const artifact of artifacts) if (!current.assets.some(a => a.name === artifact.name)) {
      verifyLocal(); gh('release', 'upload', tag, '--repo', repository, artifact.path);
    }
    current = lookup();
    checkAssets(current, true);
  }
  verifyLocal(); checkAdvance();
  if (!alreadyPublished) gh('release', 'edit', tag, '--repo', repository, '--draft=false', ...policy.latestArguments);
  const published = lookup(); verifyReleaseTarget(gh, repository, published, tag, commit, policy.prerelease);
  if (published.draft || published.assets.length !== names.length) throw Error('Published release incomplete');
  checkAssets(published, true);
  if (!policy.prerelease) await verifyStableLatest(gh, repository, published, { pause });
  return published;
}
