// SPDX-License-Identifier: MPL-2.0
export interface VideoPlacementBox { left: number; top: number; width: number; height: number }
const bottom = (box: VideoPlacementBox) => box.top + box.height;
const right = (box: VideoPlacementBox) => box.left + box.width;
const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(Math.max(min, max), value));

/** CSS translation changes position without changing ResizeObserver's size. */
export function observeVideoPlacementTransitions(node: Element, place: () => void) {
  const settled = (event: Event) => {
    if (event.target === node && ['translate', 'transform', 'width', 'height', 'top', 'right', 'bottom', 'left'].includes((event as TransitionEvent).propertyName)) place();
  };
  node.addEventListener('transitionend', settled); node.addEventListener('transitioncancel', settled);
  return () => { node.removeEventListener('transitionend', settled); node.removeEventListener('transitioncancel', settled); };
}

/** Composer is a direct child of .app and can mount after this media owner. */
export function observeVideoComposer(root: Element, place: () => void) {
  let composer: Element | null = null, stopCurrent: (() => void) | undefined, disposed = false;
  const rebind = () => {
    if (disposed) return;
    const next = root.querySelector(':scope > .composer');
    if (next === composer) return;
    stopCurrent?.(); stopCurrent = undefined; composer = next;
    if (next) {
      const changed = () => { if (!disposed && composer === next) place(); };
      const attributes = new MutationObserver(changed); attributes.observe(next, { attributes: true, attributeFilter: ['style', 'class'] });
      const size = new ResizeObserver(changed); size.observe(next);
      const stopTransition = observeVideoPlacementTransitions(next, changed);
      stopCurrent = () => { attributes.disconnect(); size.disconnect(); stopTransition(); };
    }
    place();
  };
  // Conversation streaming and drawing mutate descendants frequently. Only
  // the app's direct children can replace the Composer node we bind here.
  const children = new MutationObserver(rebind); children.observe(root, { childList: true });
  rebind();
  return () => { disposed = true; children.disconnect(); stopCurrent?.(); stopCurrent = undefined; composer = null; };
}

/** Place both independent bubbles together: object actions above the timeline.
 * Their fallbacks must not independently choose the same remaining free space.
 */
export function placeVideoToolbars(box: VideoPlacementBox, viewport: { width: number; height: number }, actionSize: { width: number; height: number }, timelineHeight: number, composer?: VideoPlacementBox, controls?: VideoPlacementBox) {
  const margin = 8, gap = 6;
  const actions = placeVideoControls(box, viewport, actionSize.height, composer, controls, { side: 'above', width: actionSize.width });
  const timeline = placeVideoControls(box, viewport, timelineHeight, composer, controls);
  const actionHeight = actionSize.height > 0 ? actionSize.height : 58;
  const trimHeight = timelineHeight > 0 ? timelineHeight : timeline.width < 520 ? 108 : 64;
  const horizontal = (left: number, width: number, other: VideoPlacementBox) => left < right(other) && left + width > other.left;
  const obstacles = [composer, controls].filter((value): value is VideoPlacementBox => Boolean(value && value.width > 0 && value.height > 0));
  const overlap = (top: number, height: number, left: number, width: number, other: VideoPlacementBox) => horizontal(left, width, other) ? Math.max(0, Math.min(top + height + gap, bottom(other)) - Math.max(top - gap, other.top)) : 0;
  const fits = (top: number, height: number, left: number, width: number) => top >= margin && top + height <= viewport.height - margin && obstacles.every(other => overlap(top, height, left, width, other) === 0);
  if (actions.top + actionHeight + gap <= timeline.top && fits(actions.top, actionHeight, actions.left, actions.width) && fits(timeline.top, trimHeight, timeline.left, timeline.width)) return { actions, timeline };
  const candidates = (height: number, preferred: number) => [preferred, margin, viewport.height - margin - height, box.top - height - gap, box.top + gap, bottom(box) + gap, bottom(box) - height - gap, ...obstacles.flatMap(other => [other.top - height - gap, bottom(other) + gap])].map(top => clamp(top, margin, viewport.height - margin - height));
  const actionTops = candidates(actionHeight, actions.top), trimTops = candidates(trimHeight, timeline.top);
  // Every edge of a free interval is considered, including an adjacent stack
  // when only one interval remains above the composer or blackboard controls.
  actionTops.push(...trimTops.map(top => top - gap - actionHeight));
  trimTops.push(...actionTops.map(top => top + actionHeight + gap));
  let best: { actionTop: number; trimTop: number; score: number } | undefined;
  for (const actionTop of actionTops) for (const trimTop of trimTops) {
    if (actionTop + actionHeight + gap > trimTop || !fits(actionTop, actionHeight, actions.left, actions.width) || !fits(trimTop, trimHeight, timeline.left, timeline.width)) continue;
    const score = Math.abs(actionTop - (box.top - actionHeight - gap)) + Math.abs(trimTop - (bottom(box) + gap));
    if (!best || score < best.score) best = { actionTop, trimTop, score };
  }
  if (best) return { actions: { ...actions, top: best.actionTop }, timeline: { ...timeline, top: best.trimTop } };
  // A tiny viewport can have less free area than both bubbles plus the
  // composer. Preserve a separate ordered stack, minimizing obstruction.
  if (actionHeight + gap + trimHeight <= viewport.height - margin * 2) {
    const tops = actionTops.map(top => clamp(top, margin, viewport.height - margin - actionHeight - gap - trimHeight));
    const cost = (top: number) => obstacles.reduce((sum, other) => sum + overlap(top, actionHeight, actions.left, actions.width, other) + overlap(top + actionHeight + gap, trimHeight, timeline.left, timeline.width, other), 0);
    const top = tops.reduce((best, top) => cost(top) < cost(best) ? top : best, tops[0] ?? margin);
    return { actions: { ...actions, top }, timeline: { ...timeline, top: top + actionHeight + gap } };
  }
  return { actions, timeline };
}

/** Prefer outside the video, then its lower content; never blindly clamp over its controls. */
export function placeVideoControls(box: VideoPlacementBox, viewport: { width: number; height: number }, measuredHeight: number, composer?: VideoPlacementBox, controls?: VideoPlacementBox, preference?: { side: 'above'; width: number }) {
  const margin = 8, gap = 6;
  const width = Math.min(preference?.width ?? Math.max(box.width, 300), 760, Math.max(120, viewport.width - margin * 2));
  const height = Number.isFinite(measuredHeight) && measuredHeight > 0 ? measuredHeight : width < 520 ? 108 : 64;
  const left = clamp(preference?.side === 'above' ? box.left : box.left + (box.width - width) / 2, margin, viewport.width - width - margin);
  const overlapsX = (other: VideoPlacementBox) => left < right(other) && left + width > other.left;
  const obstacles = [controls, composer].filter((value): value is VideoPlacementBox => Boolean(value && overlapsX(value)));
  const overlap = (top: number, other: VideoPlacementBox) => Math.max(0, Math.min(top + height + gap, bottom(other)) - Math.max(top - gap, other.top));
  const fits = (top: number) => top >= margin && top + height <= viewport.height - margin && obstacles.every(other => overlap(top, other) === 0);
  const below = controls && overlapsX(controls) ? Math.max(bottom(box), bottom(controls)) + gap : bottom(box) + gap;
  const positions = preference?.side === 'above' ? [box.top - height - gap, box.top + gap, below] : [below, bottom(box) - height - gap, box.top - height - gap];
  for (const top of positions) if (fits(top)) return { left, top, width };
  const candidates = [
    ...(composer && overlapsX(composer) ? [composer.top - height - margin, bottom(composer) + margin] : []),
    bottom(box) - height,
    ...(controls ? [bottom(controls) + gap, controls.top - height - gap] : []),
    viewport.height - height - margin, margin,
  ].map(top => clamp(top, margin, viewport.height - height - margin));
  const available = candidates.find(fits);
  if (available !== undefined) return { left, top: available, width };
  // Extremely small viewports may have no non-overlapping interval. Minimize
  // obstruction within the available viewport, prioritizing the close button.
  const score = (top: number) => obstacles.reduce((total, other) => total + overlap(top, other) * (other === controls ? 4 : 2), 0);
  const top = candidates.reduce((best, value) => score(value) < score(best) ? value : best, candidates[0] ?? margin);
  return { left, top, width };
}
