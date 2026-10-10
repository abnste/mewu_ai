// SPDX-License-Identifier: MPL-2.0
import type { ConnectionProfile, ProviderPreset } from './contracts';
import type { PluginManifest } from './plugin-contracts';

/** Browser preview uses the same real bundled manifests; native reads the
 * integrity-checked installation registry instead. No duplicate static list. */
export function bundledProviderPresets(manifests: PluginManifest[]): ProviderPreset[] {
  return manifests.flatMap(manifest => manifest.contributions.flatMap(contribution => {
    if (contribution.kind !== 'model.connection') return [];
    const template = contribution.template;
    return [{ id: template.providerId, name: contribution.title, group: template.group,
      baseUrl: template.baseUrl, model: template.model, advanced: structuredClone(template.advanced),
      searchTerms: template.searchTerms ?? '', pluginId: manifest.id, pluginRevision: 1,
      contributionId: contribution.id }];
  }));
}

/** A selected template becomes an independent user draft. Plugin lifecycle
 * never edits that draft, saved connection revisions or credential references. */
export function profileFromProviderPreset(preset: ProviderPreset, id: string, name: string): ConnectionProfile {
  return { id, name, providerId: preset.id, baseUrl: preset.baseUrl, model: preset.model,
    hasKey: false, revision: 0, credentialId: null, advanced: structuredClone(preset.advanced) };
}
