// SPDX-License-Identifier: MPL-2.0
import { createSignal } from 'solid-js';
import type { ExtensionCatalogEntry, ExtensionProject } from './extension-contracts';

interface Services {
  getExtensionProject(entry: ExtensionCatalogEntry): Promise<ExtensionProject | null>;
  getExtensionProjectImage(entry: ExtensionCatalogEntry, index: number, expectedSourceCommit: string, expectedImageSha256: string): Promise<string>;
}
const identity = (entry?: ExtensionCatalogEntry) => entry ? JSON.stringify([
  entry.id, entry.version, entry.sha256, entry.repository, entry.releaseTag, entry.assetName
]) : '';
export const projectRepositoryUrl = (repository?: string) => repository && /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)
  && repository.split('/').every(part => part !== '.' && part !== '..') ? `https://github.com/${repository}` : undefined;
// Unmounted details can still have native IPCs in flight. Keep the budget across
// owners until those actual requests settle, rather than resetting it on close.
interface ImageJob { owner: symbol; started: boolean; current(): boolean; run(): Promise<void> }
const imageQueue: ImageJob[] = [];
let activeImages = 0;
function pumpImages() {
  while (activeImages < 2 && imageQueue.length) {
    const job = imageQueue.shift()!;
    if (!job.current()) continue;
    job.started = true; activeImages++;
    void (async () => {
      try { await job.run(); }
      finally { activeImages--; pumpImages(); }
    })();
  }
}
function removeQueuedImages(owner: symbol) {
  for (let index = imageQueue.length - 1; index >= 0; index--) if (imageQueue[index].owner === owner) imageQueue.splice(index, 1);
}
export function createExtensionProjectState(api: Services) {
  const [project, setProject] = createSignal<ExtensionProject | null>();
  const [selected, setSelected] = createSignal(0), [loading, setLoading] = createSignal(false);
  const [images, setImages] = createSignal<Record<number, string>>({});
  const [pendingImages, setPendingImages] = createSignal<ReadonlySet<number>>(new Set());
  const [failedImages, setFailedImages] = createSignal<ReadonlySet<number>>(new Set());
  const [projectError, setProjectError] = createSignal(false);
  type Job = ImageJob & { generation: number; entry: ExtensionCatalogEntry; index: number; sourceCommit: string; imageSha256: string };
  let disposed = false, generation = 0, key = '', entry: ExtensionCatalogEntry | undefined;
  const owner = Symbol('project-images'), pending = new Map<string, Job>();
  const current = (epoch: number) => !disposed && epoch === generation;
  async function readImage(job: Job) {
    try {
      const url = await api.getExtensionProjectImage(job.entry, job.index, job.sourceCommit, job.imageSha256);
      if (!current(job.generation)) return;
      if (url.length > 2 * 1024 * 1024 * 4 / 3 + 128 || !['data:image/png;base64,', 'data:image/jpeg;base64,', 'data:image/webp;base64,'].some(prefix => url.startsWith(prefix))) throw new Error('Invalid screenshot');
      setImages(value => ({ ...value, [job.index]: url }));
    } catch {
      if (current(job.generation)) setFailedImages(value => new Set([...value, job.index]));
    } finally {
      pending.delete(`${job.generation}:${job.index}`);
      if (current(job.generation)) setPendingImages(value => new Set([...value].filter(index => index !== job.index)));
    }
  }
  function loadImage(index: number, priority = false) {
    if (disposed || !entry || !Number.isInteger(index) || index < 0 || index >= (project()?.screenshots.length ?? 0) || images()[index]) return;
    const jobKey = `${generation}:${index}`, old = pending.get(jobKey);
    if (old) {
      if (priority && !old.started) { const at = imageQueue.indexOf(old); if (at >= 0) { imageQueue.splice(at, 1); imageQueue.unshift(old); } }
      return;
    }
    const epoch = generation, loaded = project()!, shot = loaded.screenshots[index];
    const job: Job = { owner, generation: epoch, entry, index, sourceCommit: loaded.sourceCommit, imageSha256: shot.sha256, started: false, current: () => current(epoch), run: () => readImage(job) };
    pending.set(jobKey, job); priority ? imageQueue.unshift(job) : imageQueue.push(job);
    setFailedImages(value => new Set([...value].filter(item => item !== index)));
    setPendingImages(value => new Set([...value, index])); pumpImages();
  }
  function setEntry(value?: ExtensionCatalogEntry, force = false) {
    const next = identity(value); if (disposed || !force && next === key) return false;
    generation++; key = next; entry = value; removeQueuedImages(owner); pending.clear();
    setProject(undefined); setSelected(0); setImages({}); setPendingImages(new Set<number>()); setFailedImages(new Set<number>()); setProjectError(false); setLoading(false);
    if (value) {
      const epoch = generation; setLoading(true);
      void api.getExtensionProject(value).then(result => {
        if (!current(epoch)) return;
        if (result && (result.id !== value.id || result.version !== value.version || result.packageSha256 !== value.sha256 || !projectRepositoryUrl(result.sourceRepository) || !/^[a-f0-9]{40}$/.test(result.sourceCommit) || !Array.isArray(result.screenshots) || result.screenshots.length > 6)) throw new Error('Project identity changed');
        setProject(result); if (result?.screenshots.length) loadImage(0, true);
      }).catch(() => { if (current(epoch)) setProjectError(true); })
        .finally(() => { if (current(epoch)) setLoading(false); });
    }
    return true;
  }
  return {
    project, selected, loading, images, pendingImages, failedImages, projectError, setEntry, loadImage,
    select(index: number) { if (index >= 0 && index < (project()?.screenshots.length ?? 0)) { setSelected(index); loadImage(index, true); } },
    retry() { setEntry(entry, true); },
    dispose() { disposed = true; generation++; entry = undefined; removeQueuedImages(owner); pending.clear(); setProject(undefined); setImages({}); setPendingImages(new Set<number>()); setLoading(false); }
  };
}
