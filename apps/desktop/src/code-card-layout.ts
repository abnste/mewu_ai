// SPDX-License-Identifier: MPL-2.0
export interface CodeBox { x: number; y: number; width: number; height: number }
export function placeCodeCard(viewport: { width: number; height: number }, selection: CodeBox, size: { width: number; height: number }, occupied: CodeBox[]): CodeBox | undefined {
  const edge = 6, gap = 8, { width, height } = size;
  if (![viewport.width, viewport.height, width, height, selection.x, selection.y, selection.width, selection.height].every(Number.isFinite) || width <= 0 || height <= 0 || width > viewport.width - edge * 2 || height > viewport.height - edge * 2) return;
  const obstacles = occupied.filter(box => box.width > 0 && box.height > 0);
  const left = Math.max(edge, Math.min(viewport.width - width - edge, selection.x));
  const xs = [left, viewport.width - width - edge, edge], ys: number[] = [];
  if (obstacles[0]) ys.push(obstacles[0].y - height - gap);
  ys.push(selection.y - height - gap, selection.y + selection.height + gap);
  for (const box of obstacles) { ys.push(box.y + box.height + gap, box.y - height - gap); xs.push(box.x + box.width + gap, box.x - width - gap); }
  ys.push(selection.y + gap, edge, viewport.height - height - edge);
  for (const x of new Set(xs)) for (const y of new Set(ys)) {
    if (x < edge || y < edge || x + width > viewport.width - edge || y + height > viewport.height - edge) continue;
    if (obstacles.every(box => x + width + gap / 2 <= box.x || x - gap / 2 >= box.x + box.width || y + height + gap / 2 <= box.y || y - gap / 2 >= box.y + box.height)) return { x, y, width, height };
  }
}
