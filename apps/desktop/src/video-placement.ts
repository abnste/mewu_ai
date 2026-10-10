// SPDX-License-Identifier: MPL-2.0
export interface VideoPlacementBox { left: number; top: number; width: number; height: number }
const bottom = (box: VideoPlacementBox) => box.top + box.height;
const right = (box: VideoPlacementBox) => box.left + box.width;
const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(Math.max(min, max), value));

/** Prefer outside the video, then its lower content; never blindly clamp over its controls. */
export function placeVideoControls(box: VideoPlacementBox, viewport: { width: number; height: number }, measuredHeight: number, composer?: VideoPlacementBox, controls?: VideoPlacementBox) {
  const margin = 8, gap = 6;
  const width = Math.min(Math.max(box.width, 300), 760, Math.max(120, viewport.width - margin * 2));
  const height = Number.isFinite(measuredHeight) && measuredHeight > 0 ? measuredHeight : width < 520 ? 108 : 64;
  const left = clamp(box.left + (box.width - width) / 2, margin, viewport.width - width - margin);
  const overlapsX = (other: VideoPlacementBox) => left < right(other) && left + width > other.left;
  const obstacles = [controls, composer].filter((value): value is VideoPlacementBox => Boolean(value && overlapsX(value)));
  const overlap = (top: number, other: VideoPlacementBox) => Math.max(0, Math.min(top + height + gap, bottom(other)) - Math.max(top - gap, other.top));
  const fits = (top: number) => top >= margin && top + height <= viewport.height - margin && obstacles.every(other => overlap(top, other) === 0);
  const below = controls && overlapsX(controls) ? Math.max(bottom(box), bottom(controls)) + gap : bottom(box) + gap;
  for (const top of [below, bottom(box) - height - gap, box.top - height - gap]) if (fits(top)) return { left, top, width };
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
