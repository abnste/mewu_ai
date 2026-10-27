// SPDX-License-Identifier: MPL-2.0
// Inspect persisted media, never infer annotations from an assistant's prose.
export function annotationEvidence(state) {
  const objects = state.items.flatMap(item => [
    ...(item.imageAnnotations?.drawings || []),
    ...(item.videoAnnotations?.objects || []),
  ]);
  return {
    objectCount: objects.length,
    kinds: objects.map(object => object.kind === 'rich' ? object.rich?.kind : object.kind || object.primitive?.kind),
  };
}

export function assertNativeRun(run, connectionId) {
  if (run?.externalRuntime) throw Error('Evaluation role is bound to an external Agent; select the native engine before running this suite');
  if (run && run.connectionId !== connectionId) throw Error('Evaluation run used a different model connection');
}
