// SPDX-License-Identifier: MPL-2.0
import type { ConnectionProfile } from '../contracts';
export interface ConnectionDraft { profile: ConnectionProfile; parametersText: string; key: string; clearKey: boolean; expectedRevision?: number; isNew: boolean; dirty: boolean }
export const connectionDraft = (profile: ConnectionProfile): ConnectionDraft => ({ profile: structuredClone(profile), parametersText: JSON.stringify(profile.advanced.requestParameters, null, 2), key: '', clearKey: false, expectedRevision: profile.revision, isNew: false, dirty: false });

export function settleConnectionSave(submitted: ConnectionDraft, current: ConnectionDraft, latest?: ConnectionProfile): ConnectionDraft {
  const ownRevision = (submitted.expectedRevision ?? 0) + 1;
  if (current === submitted && latest?.revision === ownRevision) return connectionDraft(latest);
  // A later cross-window update is not the result of our save. Never adopt its
  // CAS token while keeping local edits, even if the original write succeeded.
  const ownKeySaved = submitted.clearKey ? false : Boolean(submitted.key) || submitted.profile.hasKey;
  const metadata = latest?.revision === ownRevision ? latest : { hasKey: ownKeySaved, credentialId: submitted.clearKey ? null : submitted.profile.credentialId };
  return { ...current, isNew: false, dirty: true, expectedRevision: ownRevision,
    profile: { ...current.profile, revision: ownRevision, hasKey: metadata.hasKey, credentialId: metadata.credentialId },
    key: current.key === submitted.key ? '' : current.key,
    clearKey: current.clearKey === submitted.clearKey ? false : current.clearKey,
  };
}
