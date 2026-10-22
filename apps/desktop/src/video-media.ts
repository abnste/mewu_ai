// SPDX-License-Identifier: MPL-2.0
import { LatestVideoSeek, TICKS_PER_SECOND, TrimCanceled, clampPosition, type SeekIntent, type VideoRange } from './video-trim';
export interface VideoMediaObservation { sourcePlaybackTicks?: number; sourceFramePts?: number }
interface IntervalPlayback { source: string; range: VideoRange; retained: VideoRange; active: () => boolean; ready: () => boolean; seeking: boolean; started: boolean }
interface EosReset { identity: string; generation: number; deadline: number; promise: Promise<void>; current: () => boolean; cancel: () => void; fail: () => void }
interface EosLoad { identity: string; generation: number; initialSeek: boolean; emptied: boolean }
/** The browser media clock is microseconds; persisted ranges remain 100ns. */
function inwardPlaybackTicks(ticks: number, range: VideoRange) {
  const first = Math.ceil(range.startTicks / 10), last = Math.ceil(range.endTicks / 10) - 1;
  const target = Math.max(first, Math.min(Math.ceil(ticks / 10), last)) * 10;
  if (!Number.isSafeInteger(target) || target < range.startTicks || target >= range.endTicks) throw new Error('无法定位视频');
  return target;
}
function seekSeconds(ticks: number) {
  const seconds = ticks / TICKS_PER_SECOND;
  // An exact integer microsecond can round just below itself as a double.
  // Move one representable double inward before Chromium's integer conversion,
  // rather than accepting playback before the authoritative range boundary.
  if (ticks % 10 !== 0 || Math.trunc(seconds * 1_000_000) >= ticks / 10) return seconds;
  const bits = new DataView(new ArrayBuffer(8));
  bits.setFloat64(0, seconds); bits.setBigUint64(0, bits.getBigUint64(0) + 1n);
  return bits.getFloat64(0);
}
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
  private completedRange?: { source: string; startTicks: number; endTicks: number };
  private frame?: number;
  private clockFrame?: number;
  private looping = false;
  private frameGeneration = 0;
  private sourceFramePts?: number;
  private mediaIdentity: string;
  private abortSeek?: () => void;
  private eosReset?: EosReset;
  private eosLoad?: EosLoad;
  readonly seeks: LatestVideoSeek;
  constructor(private video: HTMLVideoElement, private hooks: { identity: () => string; range: () => VideoRange | undefined; position: (ticks: number) => void; playing: (value: boolean) => void; observation?: (value: VideoMediaObservation) => void; loop?: () => boolean; error: (error: unknown) => void }) {
    this.mediaIdentity = hooks.identity();
    this.seeks = new LatestVideoSeek({ currentIdentity: hooks.identity, work: (value, signal) => this.seekActual(value, signal), presented: value => { this.boundarySeek = false; hooks.position(value); this.sample(); }, error: error => { if (!(error instanceof TrimCanceled)) hooks.error(error); } });
    video.addEventListener('timeupdate', this.sample); video.addEventListener('play', this.playing); video.addEventListener('pause', this.paused); video.addEventListener('ended', this.ended);
    video.addEventListener('seeking', this.seeking); video.addEventListener('seeked', this.seeked); video.addEventListener('loadeddata', this.seeked); video.addEventListener('emptied', this.unavailable); video.addEventListener('error', this.unavailable);
    video.addEventListener('loadedmetadata', this.mediaReady);
  }
  private sample = () => {
    if (this.disposed || this.eosReset || this.mediaIdentity !== this.hooks.identity()) return;
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
      this.completedRange = ticks >= range.endTicks ? { source: this.hooks.identity(), ...range } : undefined;
      this.pause();
      if (ticks >= range.endTicks && this.hooks.loop?.()) this.queueLoop();
      else this.requestSeek(clampPosition(ticks, range));
      return;
    }
    this.hooks.position(ticks);
  };
  private clearFrame = () => {
    this.frameGeneration++; this.sourceFramePts = undefined;
    if (this.frame !== undefined) this.video.cancelVideoFrameCallback?.(this.frame);
    this.frame = undefined; if (!this.disposed) this.hooks.observation?.({});
  };
  private eosLoadCurrent() { return !!this.eosLoad && !this.disposed && this.eosLoad.generation === this.generation && this.eosLoad.identity === this.hooks.identity(); }
  private clearEosLoad() { if (this.eosLoad && !this.eosLoad.initialSeek && !this.eosLoad.emptied) this.eosLoad = undefined; }
  private unavailable = (event?: Event) => {
    this.completedRange = undefined;
    if (event?.type === 'emptied' && this.eosLoadCurrent() && this.eosLoad!.emptied) { this.eosLoad!.emptied = false; this.clearEosLoad(); this.clearFrame(); return; }
    if (event?.type === 'error') { this.eosReset?.fail(); this.eosLoad = undefined; }
    this.pause(); this.clearFrame();
  };
  /** The initial metadata handler must not replace an owned reload's seek. */
  seekInitial(ticks: number) {
    if (this.disposed) return;
    if (this.eosLoadCurrent() && this.eosLoad!.initialSeek) { this.eosLoad!.initialSeek = false; this.clearEosLoad(); return; }
    this.seek(ticks);
  }
  private prepareEos(): EosReset | undefined {
    if (this.eosReset) return this.eosReset;
    if (this.disposed || !this.video.ended || this.video.error) return;
    const identity = this.hooks.identity(), retained = this.hooks.range(); if (!retained) return;
    const range = { ...retained };
    const completed = this.completedRange;
    // Revoke retired owners before load() resets the media and queues emptied.
    this.invalidate(); this.playOwner = undefined;
    if (completed?.source === identity && completed.startTicks === range.startTicks && completed.endTicks === range.endTicks) this.completedRange = completed;
    const generation = this.generation, deadline = performance.now() + 8000;
    let reset!: EosReset, cancel!: () => void, fail!: () => void;
    const current = () => {
      if (this.disposed || generation !== this.generation || identity !== this.hooks.identity()) return false;
      const retained = this.hooks.range(); return !!retained && retained.startTicks === range.startTicks && retained.endTicks === range.endTicks;
    };
    const promise = new Promise<void>((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout>;
      const cleanup = () => { clearTimeout(timer); this.video.removeEventListener('loadedmetadata', ready); this.video.removeEventListener('error', fail); if (this.eosReset === reset) this.eosReset = undefined; };
      cancel = () => { cleanup(); reject(new TrimCanceled()); };
      fail = () => { cleanup(); reject(new Error('无法定位视频')); };
      const ready = () => { if (!current()) { cancel(); return; } if (performance.now() >= deadline) { fail(); return; } if (this.video.readyState >= 1) { cleanup(); resolve(); } };
      timer = setTimeout(fail, Math.max(0, deadline - performance.now()));
      this.video.addEventListener('loadedmetadata', ready); this.video.addEventListener('error', fail);
    });
    reset = { identity, generation, deadline, promise, current, cancel, fail }; this.eosReset = reset;
    this.eosLoad = { identity, generation, initialSeek: true, emptied: true };
    if (!current()) cancel(); else { try { this.video.load(); } catch { fail(); } }
    return reset;
  }
  private seeking = () => { if (!this.video.seeking) return; if (this.interval && !this.interval.seeking) this.pause(); this.clearFrame(); };
  private mediaReady = () => {
    // A reload owned by this source/range/generation must retain its exact-end
    // intent. Ordinary metadata or a revoked reload still clears it.
    if (!this.eosReset?.current()) this.completedRange = undefined;
    this.mediaIdentity = this.hooks.identity(); this.clearFrame(); this.sample();
  };
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
  private ended = () => {
    if (!this.video.ended) return;
    const repeat = !this.interval && this.hooks.loop?.() && this.playOwner?.current();
    this.paused();
    if (repeat) this.queueLoop();
  };
  private queueLoop() {
    if (this.looping || this.disposed || !this.hooks.loop?.()) return;
    this.looping = true;
    const intent = this.intentGeneration, source = this.hooks.identity();
    queueMicrotask(async () => {
      try {
        if (this.disposed || intent !== this.intentGeneration || source !== this.hooks.identity() || !this.hooks.loop?.()) return;
        await this.play(this.hooks.range()?.startTicks);
      } catch (error) { if (!(error instanceof TrimCanceled) && !this.disposed) this.hooks.error(error); }
      finally { this.looping = false; }
    });
  }
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
  private async seekActual({ identity, ticks, deadline = performance.now() + 8000 }: SeekIntent, signal: AbortSignal): Promise<number> {
    const reset = this.eosReset;
    if (reset) await new Promise<void>((resolve, reject) => {
      const abort = () => { signal.removeEventListener('abort', abort); reject(new TrimCanceled()); };
      if (signal.aborted) { abort(); return; }
      signal.addEventListener('abort', abort, { once: true });
      void reset.promise.then(() => { signal.removeEventListener('abort', abort); resolve(); }, error => { signal.removeEventListener('abort', abort); reject(error); });
    });
    if (this.disposed || signal.aborted || identity !== this.hooks.identity() || (reset && !reset.current())) throw new TrimCanceled();
    const generation = this.generation;
    const boundary = this.boundarySeek;
    return new Promise((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout>;
      let targetTicks = ticks;
      const current = () => !this.disposed && !signal.aborted && generation === this.generation && identity === this.hooks.identity();
      const cleanup = () => { clearTimeout(timer); signal.removeEventListener('abort', abort); this.video.removeEventListener('loadedmetadata', ready); this.video.removeEventListener('seeked', ended); this.video.removeEventListener('error', failed); if (this.abortSeek === abort) this.abortSeek = undefined; };
      const abort = () => { cleanup(); reject(new TrimCanceled()); };
      const failed = () => { cleanup(); reject(new Error('无法定位视频')); };
      const ended = () => {
        if (!current()) { abort(); return; }
        if (performance.now() >= deadline) { failed(); return; }
        if (this.video.seeking) return;
        const actual = Math.round(this.video.currentTime * TICKS_PER_SECOND);
        if (!Number.isSafeInteger(actual) || Math.abs(actual - targetTicks) > 200_000) return;
        if (boundary && actual > targetTicks) { failed(); return; }
        cleanup(); resolve(actual);
      };
      const ready = () => {
        if (!current()) { abort(); return; }
        if (performance.now() >= deadline || this.video.error) { failed(); return; }
        // Native container duration can include an AAC tail beyond the last
        // playable video sample. HTML seeking clamps to its actual media end;
        // acknowledge that endpoint without changing the saved trim authority.
        const duration = Math.round(this.video.duration * TICKS_PER_SECOND);
        targetTicks = Number.isSafeInteger(duration) && duration >= 0 ? Math.min(ticks, duration) : ticks;
        try { this.video.currentTime = seekSeconds(targetTicks); queueMicrotask(ended); } catch { failed(); }
      };
      timer = setTimeout(failed, Math.max(0, deadline - performance.now())); this.abortSeek = abort;
      signal.addEventListener('abort', abort, { once: true });
      this.video.addEventListener('error', failed); this.video.addEventListener('seeked', ended);
      if (this.video.readyState >= 1) ready(); else this.video.addEventListener('loadedmetadata', ready, { once: true });
    });
  }
  private requestSeek(ticks: number, deadline?: number) { if (this.disposed) return; const reset = this.prepareEos(); this.boundarySeek = false; this.seeks.request({ identity: this.hooks.identity(), ticks, deadline: deadline ?? reset?.deadline ?? performance.now() + 8000 }); }
  private seekWithin(ticks: number, deadline?: number) {
    if (this.disposed) return;
    this.completedRange = undefined; if (this.interval) this.pause(); const reset = this.prepareEos();
    const range = this.hooks.range();
    // Preserve an exact endpoint scrub's intent when the browser clock rounds
    // that endpoint down. A different near-end position still clears it.
    if (range && ticks === range.endTicks) this.completedRange = { source: this.hooks.identity(), ...range };
    this.requestSeek(ticks, deadline ?? reset?.deadline);
  }
  seek(ticks: number) { this.seekWithin(ticks); }
  async seekAndWait(ticks: number, active: () => boolean, deadline?: number): Promise<number> {
    this.completedRange = undefined;
    const reset = this.prepareEos();
    const generation = this.generation, source = this.hooks.identity();
    const current = () => !this.disposed && generation === this.generation && source === this.hooks.identity() && active();
    if (!current()) throw new TrimCanceled();
    this.requestSeek(ticks, deadline ?? reset?.deadline); await this.seeks.flush(current);
    if (!current() || this.video.seeking) throw new TrimCanceled();
    return Math.round(this.video.currentTime * TICKS_PER_SECOND);
  }
  playIntent() { return this.intentGeneration; }
  hasIntervalIntent() { return Boolean(this.interval); }
  pause() {
    this.eosReset?.cancel();
    const ownedSeek = Boolean(this.interval) || this.boundarySeek;
    this.intentGeneration++; this.playGeneration++; this.interval = undefined; this.rasterHeld = false; this.abortReady?.();
    const generation = this.generation, playGeneration = this.playGeneration;
    this.video.pause();
    if (ownedSeek) { this.boundarySeek = false; this.seeks.cancel(); this.abortSeek?.(); }
    // pause events are queued, and pausing an already paused native element
    // need not emit one. Publish its observed state only for this live owner.
    if (!this.disposed && generation === this.generation && playGeneration === this.playGeneration && this.mediaIdentity === this.hooks.identity() && this.video.paused) this.hooks.playing(false);
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
    const start = inwardPlaybackTicks(range.startTicks, range);
    this.pause();
    const reset = this.prepareEos(), preparation = this.intentGeneration;
    if (reset) { await reset.promise; if (preparation !== this.intentGeneration || !reset.current() || !active()) throw new TrimCanceled(); }
    const value: IntervalPlayback = { source: this.hooks.identity(), range: { ...range }, retained: { ...retained }, active, ready, seeking: true, started: false };
    this.interval = value;
    try {
      const actual = await this.seekAndWait(start, () => this.ownsInterval(value), reset?.deadline);
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
    if (this.disposed) return; const range = this.hooks.range(); if (!range) return;
    const current = Math.round(this.video.currentTime * TICKS_PER_SECOND);
    const requested = position ?? current;
    // A completed boundary seek may land slightly before the requested time.
    // Remember actual playback completion instead of treating every position
    // near the end as finished; an explicit scrub still owns its exact intent.
    const completed = this.completedRange;
    const finished = completed?.source === this.hooks.identity() && completed.startTicks === range.startTicks && completed.endTicks === range.endTicks;
    const start = inwardPlaybackTicks((position === undefined && (this.video.ended || finished)) || requested < range.startTicks || requested >= range.endTicks ? range.startTicks : requested, range);
    this.pause(); const reset = this.prepareEos(), preparation = this.intentGeneration;
    if (reset) { await reset.promise; if (preparation !== this.intentGeneration || !reset.current()) throw new TrimCanceled(); }
    const generation = this.generation, playGeneration = ++this.playGeneration, identity = this.hooks.identity();
    const currentOwner = () => {
      if (this.disposed || generation !== this.generation || playGeneration !== this.playGeneration || identity !== this.hooks.identity()) return false;
      const retained = this.hooks.range(); return !!retained && retained.startTicks === range.startTicks && retained.endTicks === range.endTicks;
    };
    this.seekWithin(clampPosition(start, range), reset?.deadline); await this.seeks.flush(currentOwner);
    if (!currentOwner()) throw new TrimCanceled();
    const actual = Math.round(this.video.currentTime * TICKS_PER_SECOND);
    if (this.video.seeking || !Number.isSafeInteger(actual) || actual < range.startTicks || actual >= range.endTicks) throw new Error('无法定位视频');
    await this.ownedPlay(currentOwner);
  }
  invalidate() { this.completedRange = undefined; this.generation++; this.pause(); this.eosLoad = undefined; this.seeks.cancel(); this.abortSeek?.(); this.clearFrame(); }
  dispose() {
    if (this.disposed) return; this.disposed = true; this.invalidate(); this.seeks.dispose();
    if (this.frame !== undefined) this.video.cancelVideoFrameCallback?.(this.frame);
    if (this.clockFrame !== undefined) cancelAnimationFrame(this.clockFrame); this.clockFrame = undefined;
    this.video.removeEventListener('timeupdate', this.sample); this.video.removeEventListener('play', this.playing); this.video.removeEventListener('pause', this.paused); this.video.removeEventListener('ended', this.ended);
    this.video.removeEventListener('seeking', this.seeking); this.video.removeEventListener('seeked', this.seeked); this.video.removeEventListener('loadeddata', this.seeked); this.video.removeEventListener('emptied', this.unavailable); this.video.removeEventListener('error', this.unavailable);
    this.video.removeEventListener('loadedmetadata', this.mediaReady);
    this.video.removeAttribute('src'); this.video.load();
  }
}
