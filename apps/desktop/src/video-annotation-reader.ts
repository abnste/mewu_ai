// SPDX-License-Identifier: MPL-2.0
import type { SpaceItem } from './contracts';
import type { VideoAnnotationPlan, VideoAnnotationPreview, VideoAnnotationPreviewTarget } from './video-annotation-contracts';
import { videoAnnotationVisible } from './video-annotations';
export interface VideoAnnotationReadState { loading: boolean; plan?: VideoAnnotationPlan; images?: ReadonlyMap<string, string>; error?: string }
export class VideoAnnotationReader {
  private generation = 0;
  private disposed = false;
  private imageGeneration = 0;
  private currentPlan?: VideoAnnotationPlan;
  private sourceIdentity = '';
  private position?: number;
  private activeKey = '';
  private prefetched = new Set<string>();
  private images: ReadonlyMap<string, string> = new Map();
  constructor(private plan: (sceneId: string, item: SpaceItem) => Promise<VideoAnnotationPlan>,
    private preview: (target: VideoAnnotationPreviewTarget, source: string) => Promise<VideoAnnotationPreview>,
    private publish: (state: VideoAnnotationReadState) => void, private maxBytes = Math.ceil(32 * 1024 * 1024 / 3) * 4 + 48 * 32) {}
  select(sceneId: string, item: SpaceItem | undefined, sourceIdentity: string) {
    const generation = ++this.generation;
    if (this.disposed) return;
    if (this.sourceIdentity !== sourceIdentity) this.position = undefined;
    this.imageGeneration++; this.currentPlan = undefined; this.sourceIdentity = sourceIdentity; this.activeKey = ''; this.prefetched.clear(); this.images = new Map();
    this.publish({ loading: Boolean(item?.videoAnnotations) });
    if (!item?.videoAnnotations) return;
    const captured = structuredClone(item), active = () => !this.disposed && this.generation === generation;
    void (async () => {
      const plan = await this.plan(sceneId, captured); if (!active()) return;
      // at() publishes the complete active-set transition. Do not briefly
      // signal ready between the plan and its first required raster set.
      this.currentPlan = plan; this.at(this.position);
    })().catch(cause => { if (active()) this.publish({ loading: false, error: cause instanceof Error ? cause.message : String(cause) }); });
  }
  at(position?: number) {
    this.position = position;
    const plan = this.currentPlan; if (!plan || this.disposed) return;
    const entries = plan.entries.filter(entry => videoAnnotationVisible(entry.interval, plan.range, position)), key = JSON.stringify(entries.map(entry => entry.annotationId));
    if (key !== this.activeKey) {
      this.activeKey = key; const generation = this.generation, imageGeneration = ++this.imageGeneration;
      this.images = new Map(); this.publish({ loading: Boolean(entries.length), plan, images: this.images });
      const active = () => !this.disposed && generation === this.generation && imageGeneration === this.imageGeneration;
      void (async () => {
        const images = new Map<string, string>(); let bytes = 0;
        for (const entry of entries) {
          const preview = await this.preview({ target: plan.target, documentSha256: plan.documentSha256, annotationId: entry.annotationId, reference: entry.reference }, this.sourceIdentity);
          if (!active()) return;
          bytes += preview.dataUrl.length; if (bytes > this.maxBytes) throw Error('视频标注预览过大');
          images.set(entry.annotationId, preview.dataUrl);
        }
        if (active()) { this.images = images; this.publish({ loading: false, plan, images }); }
      })().catch(cause => { if (active()) this.publish({ loading: false, plan, error: cause instanceof Error ? cause.message : String(cause) }); });
    }
    // Only prefetch bytes for the next second, never mount hidden <image>s.
    // The shared broker bounds ready bytes; no component retains these results.
    if (position !== undefined && Number.isSafeInteger(position)) for (const entry of plan.entries) {
      if (entry.interval.startTicks <= position || entry.interval.startTicks > position + 10_000_000 || entry.interval.startTicks >= plan.range.endTicks || this.prefetched.has(entry.annotationId)) continue;
      this.prefetched.add(entry.annotationId);
      void this.preview({ target: plan.target, documentSha256: plan.documentSha256, annotationId: entry.annotationId, reference: entry.reference }, this.sourceIdentity).catch(() => {});
    }
  }
  imageFailed(id: string, dataUrl: string) {
    if (!this.disposed && this.images.get(id) === dataUrl) { this.imageGeneration++; this.images = new Map(); this.publish({ loading: false, plan: this.currentPlan, error: '无法读取视频标注' }); }
  }
  dispose() { this.generation++; this.imageGeneration++; this.disposed = true; this.currentPlan = undefined; this.images = new Map(); }
}
