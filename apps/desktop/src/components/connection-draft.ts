// SPDX-License-Identifier: MPL-2.0
import type { ConnectionProfile } from '../contracts';
import { validateConnectionProfile } from '../connection-policy';
export interface ConnectionDraft { profile: ConnectionProfile; parametersText: string; key: string; keyLoaded: boolean; keyEdited: boolean; clearKey: boolean; expectedRevision?: number; isNew: boolean; dirty: boolean }
export const connectionDraft = (profile: ConnectionProfile): ConnectionDraft => ({ profile: structuredClone(profile), parametersText: JSON.stringify(profile.advanced.requestParameters, null, 2), key: '', keyLoaded: !profile.hasKey, keyEdited: false, clearKey: false, expectedRevision: profile.revision, isNew: false, dirty: false });

/** Discovery changes only this exact visible draft; saving remains explicit. */
export function applyModelCatalogToDraft(submitted: ConnectionDraft, current: ConnectionDraft, resolvedBaseUrl: string): ConnectionDraft {
  if (submitted !== current) return current;
  const previous = submitted.profile.baseUrl.trim().replace(/\/+$/, '');
  if (resolvedBaseUrl === previous) return current;
  const profile = { ...submitted.profile, baseUrl: resolvedBaseUrl };
  validateConnectionProfile(profile, false);
  const from = new URL(submitted.profile.baseUrl), to = new URL(resolvedBaseUrl);
  if (from.pathname !== '/' || submitted.profile.advanced.requestPath?.trim() || from.origin !== to.origin || !['/v1', '/api/v1'].includes(to.pathname)) throw Error('模型目录返回的 API 地址无效');
  return { ...submitted, profile, dirty: true };
}

export function settleConnectionSave(submitted: ConnectionDraft, current: ConnectionDraft, latest?: ConnectionProfile): ConnectionDraft {
  const ownRevision = (submitted.expectedRevision ?? 0) + 1;
  if (latest?.revision === ownRevision && (current === submitted || (!current.dirty && !current.isNew && current.expectedRevision === ownRevision))) return connectionDraft(latest);
  // A later cross-window update is not the result of our save. Never adopt its
  // CAS token while keeping local edits, even if the original write succeeded.
  const ownKeySaved = submitted.clearKey ? false : Boolean(submitted.key) || submitted.profile.hasKey;
  const metadata = latest?.revision === ownRevision ? latest : { hasKey: ownKeySaved, credentialId: submitted.clearKey ? null : submitted.profile.credentialId };
  return { ...current, isNew: false, dirty: true, expectedRevision: ownRevision,
    profile: { ...current.profile, revision: ownRevision, hasKey: metadata.hasKey, credentialId: metadata.credentialId },
    key: current.key === submitted.key ? '' : current.key,
    keyLoaded: current.key !== submitted.key && current.keyLoaded,
    keyEdited: current.key !== submitted.key && current.keyEdited,
    clearKey: current.clearKey === submitted.clearKey ? false : current.clearKey,
  };
}
