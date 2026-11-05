// SPDX-License-Identifier: MPL-2.0
import { createResource, ErrorBoundary, Show } from 'solid-js';
import { CircleAlert, LoaderCircle } from 'lucide-solid';
import App from './App';
import * as bridge from './bridge';
import { t } from './i18n';

async function firstSnapshot() {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const snapshot = await Promise.race([
      bridge.getSnapshot(),
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error(t('未收到截图空间状态'))), 8000);
      }),
    ]);
    if (!snapshot.scenes.some(scene => scene.id === snapshot.activeSceneId)) {
      throw new Error(t('未收到截图空间状态'));
    }
    return snapshot;
  } finally {
    clearTimeout(timer);
  }
}

export default function SpaceStartup() {
  // Build the scene subtree with an authoritative scene on its first render.
  // Listener registration in App still re-reads changes made during mounting.
  const [initial] = createResource(firstSnapshot);
  return <ErrorBoundary fallback={cause => <div class="startup-state" role="alert">
    <CircleAlert size={19}/><span>{cause instanceof Error ? cause.message : String(cause)}</span>
    <button onClick={() => location.reload()}>{t('重新载入')}</button>
  </div>}>
    <Show when={initial()} keyed fallback={<div class="startup-state"><LoaderCircle size={20} class="spin"/></div>}>
      {snapshot => <App initialSnapshot={snapshot}/>}
    </Show>
  </ErrorBoundary>;
}
