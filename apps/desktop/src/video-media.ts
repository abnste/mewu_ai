// SPDX-License-Identifier: MPL-2.0
import { LatestVideoSeek, TICKS_PER_SECOND, TrimCanceled, clampPosition, type VideoRange } from './video-trim';
export interface VideoMediaObservation { sourcePlaybackTicks?: number; sourceFramePts?: number }
interface IntervalPlayback { source: string; range: VideoRange; retained: VideoRange; active: () => boolean; ready: () => boolean; seeking: boolean; started: boolean }
/** A stable media element. Range edits never replace its src or node. */
export class VideoPlayback {
  private disposed = false;
  private generation = 0;
  private playGeneration = 0;
  private intentGeneration = 0;
  private interval?: IntervalPlayback;
  private rasterHeld = false;
  private playOwner?: { current: () => boolean };
  private abortReady?: () => void;
  private boundarySeek = false;
  private frame?: number;
  private clockFrame?: number;
  private frameGeneration = 0;
  private sourceFramePts?: number;
  private mediaIdentity: string;
  private abortSeek?: () => void;
  readonly seeks: LatestVideoSeek;
  constructor(private video: HTMLVideoElement, private hooks: { identity: () => string; range: () => VideoRange | undefined; position: (ticks: number) => void; playing: (value: boolean) => void; observation?: (value: VideoMediaObservation) => void; error: (error: unknown) => void }) {
    this.mediaIdentity = hooks.identity();
    this.seeks = new LatestVideoSeek({ currentIdentity: hooks.identity, work: value => this.seekActual(value.identity, value.ticks), presented: value => { this.boundarySeek = false; hooks.position(value); this.sample(); }, error: error => { if (!(error instanceof TrimCanceled)) hooks.error(error); } });
    video.addEventListener('timeupdate', this.sample); video.addEventListener('play', this.playing); video.addEventListener('pause', this.paused); video.addEventListener('ended', this.paused);
    video.addEventListener('seeking', this.seeking); video.addEventListener('seeked', this.seeked); video.addEventListener('loadeddata', this.seeked); video.addEventListener('emptied', this.unavailable); video.addEventListener('error', this.unavailable);
    video.addEventListener('loadedmetadata', this.mediaReady);
  }
  private sample = () => {
    if (this.disposed || this.mediaIdentity !== this.hooks.identity()) return;
    const ticks = Math.round(this.video.currentTime * TICKS_PER_SECOND), range = this.hooks.range();
    if (!Number.isSafeInteger(ticks) || ticks < 0) { this.hooks.observation?.({}); return; }
    const interval = this.interval;
    if (interval && (!this.ownsInterval(interval) || (!interval.seeking && (ticks < interval.range.startTicks || ticks >= interval.range.endTicks)))) {
      const finished = this.ownsInterval(interval) && ticks >= interval.range.endTicks;
      this.pause();
      if (finished) { this.requestSeek(interval.range.endTicks); this.boundarySeek = true; }
      return;
    }
    // The immutable original Asset is the media URL: currentTime already uses
    // source playback time. Trim start is never added. PTS is separate evidence.
    this.hooks.observation?.(!this.video.seeking && this.video.readyState >= 2 ? { sourcePlaybackTicks: ticks, sourceFramePts: this.sourceFramePts } : {});
    if (!this.video.paused && range && (ticks < range.startTicks || ticks >= range.endTicks)) {
      this.pause(); this.seek(clampPosition(ticks, range)); return;
    }
    this.hooks.position(ticks);
  };
  private clearFrame = () => {
    this.frameGeneration++; this.sourceFramePts = undefined;
    if (this.frame !== undefined) this.video.cancelVideoFrameCallback?.(this.frame);
    this.frame = undefined; if (!this.disposed) this.hooks.observation?.({});
  };
  private unavailable = () => { this.pause(); this.clearFrame(); };
  private seeking = () => { if (!this.video.seeking) return; if (this.interval && !this.interval.seeking) this.pause(); this.clearFrame(); };
  private mediaReady = () => { this.mediaIdentity = this.hooks.identity(); this.clearFrame(); this.sample(); };
  private seeked = () => { this.sample(); this.frames(true); };
  private playing = () => {
    if (this.video.paused) return;
    if (this.playOwner && !this.playOwner.current()) { this.video.pause(); return; }
    this.hooks.playing(true); this.sample(); this.frames(); this.clockFrames();
  };
  private paused = () => {
    // Media events are queued tasks. An older pause must not erase a newer
    // play, or revoke the next interval while it is still seeking/loading.
    if (!this.video.paused) return;
    if (this.interval?.started && !this.rasterHeld) this.pause();
    this.hooks.playing(false);
    if (this.frame !== undefined) this.video.cancelVideoFrameCallback?.(this.frame); this.frame = undefined;
    if (this.clockFrame !== undefined) cancelAnimationFrame(this.clockFrame); this.clockFrame = undefined;
    this.sample();
  };
  private clockFrames() {
    if (this.disposed || this.video.paused || this.clockFrame !== undefined || typeof requestAnimationFrame !== 'function') return;
    const generation = this.generation;
    const id = requestAnimationFrame(() => {
      if (this.disposed || generation !== this.generation || this.clockFrame !== id) return;
      this.clockFrame = undefined;
      // Held source frames can span a short annotation interval. Read the real
      // media clock; do not extrapolate from wall time or wait for a new PTS.
      this.sample(); this.clockFrames();
    });
    this.clockFrame = id;
  }
  private frames(once = false) {
    if (this.disposed || this.mediaIdentity !== this.hooks.identity() || (!once && this.video.paused) || !this.video.requestVideoFrameCallback || this.frame !== undefined) return;
    const generation = this.frameGeneration, source = this.hooks.identity();
    const id = this.video.requestVideoFrameCallback((_now, metadata) => {
      if (this.disposed || generation !== this.frameGeneration || source !== this.hooks.identity() || this.frame !== id) return;
      this.frame = undefined;
      const pts = Math.round(metadata.mediaTime * TICKS_PER_SECOND);
      this.sourceFramePts = Number.isSafeInteger(pts) && pts >= 0 && !this.video.seeking ? pts : undefined;
      this.sample(); this.frames();
    });
    this.frame = id;
  }
  private seekActual(identity: string, ticks: number): Promise<number> {
    const generation = this.generation;
    return new Promise((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout>;
      const current = () => !this.disposed && generation === this.generation && identity === this.hooks.identity();
      const cleanup = () => { clearTimeout(timer); this.video.removeEventListener('loadedmetadata', ready); this.video.removeEventListener('seeked', ended); this.video.removeEventListener('error', failed); if (this.abortSeek === abort) this.abortSeek = undefined; };
      const abort = () => { cleanup(); reject(new TrimCanceled()); };
      const failed = () => { cleanup(); reject(new Error('无法定位视频')); };
      const ended = () => {
        if (!current()) { abort(); return; }
        if (this.video.seeking) return;
        const actual = Math.round(this.video.currentTime * TICKS_PER_SECOND);
        if (!Number.isSafeInteger(actual) || Math.abs(actual - ticks) > 200_000) return;
        if (this.boundarySeek && actual > ticks) { failed(); return; }
        cleanup(); resolve(actual);
      };
      const ready = () => { if (!current()) { abort(); return; } try { this.video.currentTime = ticks / TICKS_PER_SECOND; queueMicrotask(ended); } catch { failed(); } };
      timer = setTimeout(failed, 8000); this.abortSeek = abort;
      this.video.addEventListener('error', failed); this.video.addEventListener('seeked', ended);
      if (this.video.readyState >= 1) ready(); else this.video.addEventListener('loadedmetadata', ready, { once: true });
    });
  }
  private requestSeek(ticks: number) { this.boundarySeek = false; this.seeks.request({ identity: this.hooks.identity(), ticks }); }
  seek(ticks: number) { if (this.interval) this.pause(); this.requestSeek(ticks); }
  async seekAndWait(ticks: number, active: () => boolean): Promise<number> {
    const generation = this.generation, source = this.hooks.identity();
    const current = () => !this.disposed && generation === this.generation && source === this.hooks.identity() && active();
    if (!current()) throw new TrimCanceled();
    this.requestSeek(ticks); await this.seeks.flush(current);
    if (!current() || this.video.seeking) throw new TrimCanceled();
    return Math.round(this.video.currentTime * TICKS_PER_SECOND);
  }
  playIntent() { return this.intentGeneration; }
  hasIntervalIntent() { return Boolean(this.interval); }
  pause() {
    const ownedSeek = Boolean(this.interval) || this.boundarySeek;
    this.intentGeneration++; this.playGeneration++; this.interval = undefined; this.rasterHeld = false; this.abortReady?.(); this.video.pause();
    if (ownedSeek) { this.boundarySeek = false; this.seeks.cancel(); this.abortSeek?.(); }
  }
  holdForRaster() { this.rasterHeld = true; this.video.pause(); }
  private ownsInterval(value: IntervalPlayback) {
    const range = this.hooks.range();
    return !this.disposed && this.interval === value && value.source === this.hooks.identity() && value.active() && !!range && range.startTicks === value.retained.startTicks && range.endTicks === value.retained.endTicks;
  }
  private async ownedPlay(current: () => boolean) {
    const owner = { current }; this.playOwner = owner; this.rasterHeld = false;
    try { await this.video.play(); }
    catch (error) {
      if (current() && this.rasterHeld) return;
      if (!current()) { if (this.playOwner === owner) this.video.pause(); throw new TrimCanceled(); }
      throw error;
    }
    if (!current()) { if (this.playOwner === owner) this.video.pause(); throw new TrimCanceled(); }
  }
  private waitForRaster(value: IntervalPlayback): Promise<void> {
    return new Promise((resolve, reject) => {
      let poll: ReturnType<typeof setTimeout>, timeout: ReturnType<typeof setTimeout>;
      const cleanup = () => { clearTimeout(poll); clearTimeout(timeout); if (this.abortReady === abort) this.abortReady = undefined; };
      const abort = () => { cleanup(); reject(new TrimCanceled()); };
      const check = () => {
        if (!this.ownsInterval(value)) { abort(); return; }
        if (value.ready()) { cleanup(); resolve(); return; }
        poll = setTimeout(check, 16);
      };
      this.abortReady = abort;
      timeout = setTimeout(() => { cleanup(); reject(Error('视频标注预览未就绪')); }, 8000);
      check();
    });
  }
  async playInterval(range: VideoRange, active: () => boolean, ready: () => boolean): Promise<number> {
    const retained = this.hooks.range();
    if (!retained || !active() || !Number.isSafeInteger(range.startTicks) || !Number.isSafeInteger(range.endTicks) || range.startTicks < retained.startTicks || range.endTicks > retained.endTicks || range.startTicks >= range.endTicks) throw new TrimCanceled();
    this.pause();
    const value: IntervalPlayback = { source: this.hooks.identity(), range: { ...range }, retained: { ...retained }, active, ready, seeking: true, started: false };
    this.interval = value;
    try {
      const actual = await this.seekAndWait(range.startTicks, () => this.ownsInterval(value));
      if (!this.ownsInterval(value)) throw new TrimCanceled();
      if (actual < range.startTicks || actual >= range.endTicks) throw Error('未能定位到视频标注');
      value.seeking = false;
      await this.waitForRaster(value);
      await this.resumeCurrent();
      return actual;
    } catch (error) { if (this.interval === value) this.pause(); throw error; }
  }
  async resumeCurrent() {
    const range = this.hooks.range(), ticks = Math.round(this.video.currentTime * TICKS_PER_SECOND);
    const interval = this.interval;
    if (this.disposed || this.video.seeking || !range || ticks < range.startTicks || ticks >= range.endTicks || (interval && (!this.ownsInterval(interval) || ticks < interval.range.startTicks || ticks >= interval.range.endTicks || !interval.ready()))) throw new TrimCanceled();
    // Raster loading only held the current media position; do not issue a seek.
    const generation = this.generation, playGeneration = ++this.playGeneration, identity = this.hooks.identity();
    if (interval) interval.started = true;
    await this.ownedPlay(() => !this.disposed && generation === this.generation && identity === this.hooks.identity() && playGeneration === this.playGeneration && (!interval || this.ownsInterval(interval)));
  }
  async play(position?: number) {
    this.pause();
    const generation = this.generation, playGeneration = ++this.playGeneration, identity = this.hooks.identity(), range = this.hooks.range(); if (!range || this.disposed) return;
    const current = Math.round(this.video.currentTime * TICKS_PER_SECOND);
    const requested = position ?? current;
    // Both the play button and a resumed timeline gesture can supply the
    // exclusive endpoint. Replay the retained interval rather than seek/play
    // at its end and immediately stop. Native ended may precede rounded ticks.
    const start = (position === undefined && this.video.ended) || requested < range.startTicks || requested >= range.endTicks ? range.startTicks : requested;
    this.seek(clampPosition(start, range)); await this.seeks.flush(() => !this.disposed && generation === this.generation && playGeneration === this.playGeneration && identity === this.hooks.identity());
    if (this.disposed || generation !== this.generation || playGeneration !== this.playGeneration || identity !== this.hooks.identity()) throw new TrimCanceled();
    await this.ownedPlay(() => !this.disposed && generation === this.generation && playGeneration === this.playGeneration && identity === this.hooks.identity());
  }
  invalidate() { this.generation++; this.pause(); this.seeks.cancel(); this.abortSeek?.(); this.clearFrame(); }
  dispose() {
    if (this.disposed) return; this.invalidate(); this.disposed = true; this.seeks.dispose();
    if (this.frame !== undefined) this.video.cancelVideoFrameCallback?.(this.frame);
    this.video.removeEventListener('timeupdate', this.sample); this.video.removeEventListener('play', this.playing); this.video.removeEventListener('pause', this.paused); this.video.removeEventListener('ended', this.paused);
    this.video.removeEventListener('seeking', this.seeking); this.video.removeEventListener('seeked', this.seeked); this.video.removeEventListener('loadeddata', this.seeked); this.video.removeEventListener('emptied', this.unavailable); this.video.removeEventListener('error', this.unavailable);
    this.video.removeEventListener('loadedmetadata', this.mediaReady);
    this.video.removeAttribute('src'); this.video.load();
  }
}
