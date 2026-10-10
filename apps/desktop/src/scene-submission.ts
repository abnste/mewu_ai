// SPDX-License-Identifier: MPL-2.0
import type { Reference, SceneCommand, Snapshot } from './contracts';

/** Run inside one queue entry so draft autosave cannot interleave with begin_run. */
export async function submitSceneDraft(input: { sceneId: string; draft: string; refs: Reference[] }, hooks: {
  save: (command: SceneCommand) => Promise<void>;
  send: (sceneId: string) => Promise<Snapshot>;
}): Promise<Snapshot> {
  const references = input.refs.map(value => ({ ...value }));
  await hooks.save({ type: 'set_draft', sceneId: input.sceneId, draft: input.draft });
  await hooks.save({ type: 'set_refs', sceneId: input.sceneId, refs: references });
  return hooks.send(input.sceneId);
}
