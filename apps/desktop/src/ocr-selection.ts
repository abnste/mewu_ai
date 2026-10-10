// SPDX-License-Identifier: MPL-2.0
import type { OcrDocument, OcrTarget } from './contracts';

export interface OcrGlyph { x: number; y: number; width: number; height: number; start: number; end: number }
export interface OcrLayout { text: string; glyphs: OcrGlyph[] }
const segmenter = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

// Keep native OCR reading order. Geometry is used for hit testing, never to reorder text.
// As in the WPF OcrSelectionLayout, each grapheme occupies an equal share of its word box.
export function ocrLayout(document: OcrDocument): OcrLayout {
  const glyphs: OcrGlyph[] = [];
  let text = '';
  for (const [lineIndex, line] of document.lines.entries()) {
    if (lineIndex) text += '\n';
    const lineStart = text.length;
    text += line.text;
    let offset = 0;
    for (const word of line.words) {
      if (!word.text || ![word.x, word.y, word.width, word.height].every(Number.isFinite) || word.width <= 0 || word.height <= 0) continue;
      const match = line.text.indexOf(word.text, offset);
      if (match < 0) continue;
      offset = match + word.text.length;
      const parts = Array.from(segmenter.segment(word.text));
      for (let index = 0; index < parts.length; index++) {
        const part = parts[index], start = lineStart + match + part.index;
        glyphs.push({ x: word.x + word.width * index / parts.length, y: word.y, width: word.width / parts.length, height: word.height, start, end: start + part.segment.length });
      }
    }
  }
  return { text, glyphs };
}

export function nearestOcrGlyph(glyphs: OcrGlyph[], x: number, y: number): number {
  let nearest = -1, distance = Infinity;
  glyphs.forEach((glyph, index) => {
    const dx = Math.max(glyph.x - x, 0, x - glyph.x - glyph.width);
    const dy = Math.max(glyph.y - y, 0, y - glyph.y - glyph.height);
    const next = dx * dx + dy * dy;
    if (next < distance) { nearest = index; distance = next; }
  });
  return nearest;
}

// Windows OCR returns unrotated boxes; the official sample rotates the overlay by +TextAngle.
export function unrotateOcrPoint(x: number, y: number, width: number, height: number, angle = 0) {
  const radians = -angle * Math.PI / 180, dx = x - width / 2, dy = y - height / 2;
  return { x: width / 2 + dx * Math.cos(radians) - dy * Math.sin(radians), y: height / 2 + dx * Math.sin(radians) + dy * Math.cos(radians) };
}

export function sameOcrTarget(left: OcrTarget, right: OcrTarget) {
  return left.sceneId === right.sceneId && left.regionId === right.regionId && left.backgroundId === right.backgroundId && left.drawingRevision === right.drawingRevision && left.x === right.x && left.y === right.y && left.width === right.width && left.height === right.height;
}
