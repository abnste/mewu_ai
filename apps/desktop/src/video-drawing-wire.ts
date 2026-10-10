// SPDX-License-Identifier: MPL-2.0
import type { Drawing } from './contracts';
import type { VideoAnnotationTarget, VideoPixelPoint, VideoPixelRect } from './video-annotation-contracts';
import type { VideoDrawingAction, VideoDrawingGrant, VideoDrawingNativeArgs, VideoLocalVectorContent, VideoSourceDrawing, VideoSourceVector, VideoVectorLayoutRef, VideoVectorRead } from './video-drawing-contracts';

const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const keys = (value: Record<string, unknown>, expected: string[]) => Object.keys(value).length === expected.length && expected.every(key => Object.hasOwn(value, key));
export const videoDrawingUuid = (value: unknown): value is string => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value);
const hash = (value: unknown): value is string => typeof value === 'string' && /^[0-9a-f]{64}$/.test(value);
const color = (value: unknown) => typeof value === 'string' && /^#[a-f\d]{6}$/i.test(value);
const revision = (value: unknown) => Number.isSafeInteger(value) && Number(value) >= 0;
const point = (value: unknown): value is VideoPixelPoint => object(value) && keys(value, ['x', 'y']) && typeof value.x === 'number' && typeof value.y === 'number' && Number.isFinite(value.x) && Number.isFinite(value.y);
const rect = (value: unknown): value is VideoPixelRect => object(value) && keys(value, ['x', 'y', 'width', 'height']) && [value.x, value.y, value.width, value.height].every(value => typeof value === 'number' && Number.isFinite(value)) && Number(value.width) >= 0 && Number(value.height) >= 0;
const encoded = (value: unknown) => new TextEncoder().encode(JSON.stringify(value)).length;
const sameTarget = (a: VideoAnnotationTarget, b: VideoAnnotationTarget) => a.sceneId === b.sceneId && a.itemId === b.itemId && a.sourceId === b.sourceId && a.expectedRangeRevision === b.expectedRangeRevision && a.expectedAnnotationRevision === b.expectedAnnotationRevision;
const sameRect = (a: VideoPixelRect, b: VideoPixelRect) => a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
const sameReference = (a: VideoVectorLayoutRef, b: VideoVectorLayoutRef) => a.layoutId === b.layoutId && a.layoutSha256 === b.layoutSha256 && a.rasterSha256 === b.rasterSha256 && a.width === b.width && a.height === b.height && sameRect(a.geometryBounds, b.geometryBounds);
export function validVideoDrawingTarget(value: unknown): value is VideoAnnotationTarget {
  return object(value) && keys(value, ['sceneId', 'itemId', 'sourceId', 'expectedRangeRevision', 'expectedAnnotationRevision']) && [value.sceneId, value.itemId, value.sourceId].every(videoDrawingUuid) && revision(value.expectedRangeRevision) && revision(value.expectedAnnotationRevision);
}
export function validVideoVectorReference(value: unknown): value is VideoVectorLayoutRef {
  if (!object(value) || !keys(value, ['layoutId', 'layoutSha256', 'rasterSha256', 'width', 'height', 'geometryBounds']) || !videoDrawingUuid(value.layoutId) || !hash(value.layoutSha256) || !hash(value.rasterSha256) || ![value.width, value.height].every(value => Number.isSafeInteger(value) && Number(value) > 0 && Number(value) <= 6000) || Number(value.width) * Number(value.height) > 4_000_000 || !rect(value.geometryBounds)) return false;
  const box = value.geometryBounds;
  return box.x >= 0 && box.y >= 0 && box.x + box.width <= Number(value.width) && box.y + box.height <= Number(value.height) && box.x <= 256 && box.y <= 256 && Number(value.width)-box.x-box.width <=256 && Number(value.height)-box.y-box.height <=256;
}
function validVector(value: unknown, pointsKey: 'sourcePoints' | 'localPoints'): boolean {
  if (!object(value) || value.version !== 1 || !color(value.color)) return false;
  const number = value.kind === 'number';
  if (!keys(value, number ? ['kind', 'version', pointsKey, 'color', 'number', 'diameter'] : ['kind', 'version', pointsKey, 'color', 'strokeWidth'])) return false;
  const points = value[pointsKey];
  if (!Array.isArray(points) || !points.length || points.length > 4096 || !points.every(point)) return false;
  if (number) return points.length === 1 && Number.isInteger(value.number) && Number(value.number) >= 1 && Number(value.number) <= 9999 && typeof value.diameter === 'number' && Number.isFinite(value.diameter) && value.diameter >= 22 && value.diameter <= 64;
  if (!['pen', 'line', 'arrow', 'rect', 'ellipse'].includes(String(value.kind)) || typeof value.strokeWidth !== 'number' || !Number.isFinite(value.strokeWidth) || value.strokeWidth < .5 || value.strokeWidth > 64 || value.kind !== 'pen' && points.length !== 2) return false;
  if (value.kind === 'line' || value.kind === 'arrow') return points[0].x !== points[1].x || points[0].y !== points[1].y;
  if (value.kind === 'rect' || value.kind === 'ellipse') return points[0].x !== points[1].x && points[0].y !== points[1].y;
  return true;
}
export function validVideoSourceDrawing(value: unknown): value is VideoSourceDrawing {
  if (!object(value)) return false;
  if (value.kind !== 'text') return validVector(value, 'sourcePoints') && encoded(value) <= 256 * 1024;
  const content = value.content;
  return keys(value, ['kind', 'topLeft', 'content']) && point(value.topLeft) && object(content) && keys(content, ['version', 'text', 'color', 'fontSize']) && content.version === 1 && typeof content.text === 'string' && content.text.trim().length > 0 && [...content.text].length <= 500 && new TextEncoder().encode(content.text).length <= 4096 && !/[\x00-\x08\x0b-\x1f\x7f-\x9f]/.test(content.text) && color(content.color) && typeof content.fontSize === 'number' && Number.isFinite(content.fontSize) && content.fontSize >= 8 && content.fontSize <= 128;
}
export function drawingToVideoSource(drawing: Drawing): VideoSourceDrawing {
  if (drawing.origin || drawing.rich) throw Error('视频绘制对象无效');
  const points = drawing.points.map(point => ({ ...point }));
  const value: unknown = drawing.kind === 'text' ? { kind: 'text', topLeft: points[0], content: { version: 1, text: drawing.text, color: drawing.color, fontSize: drawing.fontSize } }
    : drawing.kind === 'number' ? { kind: 'number', version: 1, sourcePoints: points, color: drawing.color, number: Number(drawing.text), diameter: drawing.fontSize }
    : { kind: drawing.kind, version: 1, sourcePoints: points, color: drawing.color, strokeWidth: drawing.strokeWidth };
  if (drawing.kind === 'text' && points.length !== 1 || drawing.kind !== 'text' && drawing.kind !== 'number' && (drawing.text !== undefined || drawing.fontSize !== undefined) || drawing.kind === 'number' && !/^[1-9]\d{0,3}$/.test(drawing.text ?? '') || !validVideoSourceDrawing(value)) throw Error('视频绘制对象无效');
  return value;
}
export function validateVideoVectorRead(raw: unknown, expected: Omit<VideoVectorRead, 'content'>): VideoVectorRead {
  if (!object(raw) || !keys(raw, ['requestId', 'target', 'annotationId', 'reference', 'content']) || raw.requestId !== expected.requestId || raw.annotationId !== expected.annotationId || !validVideoDrawingTarget(raw.target) || !sameTarget(raw.target, expected.target) || !validVideoVectorReference(raw.reference) || !sameReference(raw.reference, expected.reference) || !validVector(raw.content, 'localPoints') || encoded(raw.content) > 256 * 1024) throw Error('视频绘制对象已变化');
  const content = raw.content as VideoLocalVectorContent, points = content.localPoints, reference = raw.reference;
  if (points.some(point => point.x < 0 || point.y < 0 || point.x > reference.width || point.y > reference.height)) throw Error('视频绘制对象无效');
  const xs = points.map(point => point.x), ys = points.map(point => point.y), geometry = { x: Math.min(...xs), y: Math.min(...ys), width: Math.max(...xs) - Math.min(...xs), height: Math.max(...ys) - Math.min(...ys) };
  if (content.kind === 'number') geometry.width = geometry.height = content.diameter;
  if (!sameRect(geometry, reference.geometryBounds)) throw Error('视频绘制对象无效');
  return structuredClone(raw) as unknown as VideoVectorRead;
}
export function localVectorDrawing(reply: VideoVectorRead, topLeft: VideoPixelPoint): Drawing {
  const content = reply.content, points = content.localPoints.map(point => ({ x: point.x + topLeft.x, y: point.y + topLeft.y }));
  return content.kind === 'number' ? { id: reply.annotationId, kind: 'number', points, color: content.color, strokeWidth: 1, text: String(content.number), fontSize: content.diameter }
    : { id: reply.annotationId, kind: content.kind, points, color: content.color, strokeWidth: content.strokeWidth };
}
export function videoDrawingNativeArgs(requestId: string, target: VideoAnnotationTarget, grant: VideoDrawingGrant, action: VideoDrawingAction): VideoDrawingNativeArgs {
  const raw: unknown = action;
  if (!object(raw) || (raw.type !== 'add' || !keys(raw, ['type', 'content'])) && (raw.type !== 'update' || !keys(raw, ['type', 'annotationId', 'reference', 'content']))) throw Error('视频绘制对象无效');
  if (!videoDrawingUuid(requestId) || !validVideoDrawingTarget(target) || !grant || typeof grant.pluginId !== 'string' || !grant.pluginId || typeof grant.contributionId !== 'string' || !grant.contributionId || !revision(grant.revision) || grant.revision < 1 || !Array.isArray(grant.tools) || !validVideoSourceDrawing(action.content) || !grant.tools.includes(action.content.kind)) throw Error('视频绘制工具不可用');
  if (action.type === 'update' && (!videoDrawingUuid(action.annotationId) || !validVideoVectorReference(action.reference) || raw.content && object(raw.content) && raw.content.kind === 'text')) throw Error('视频绘制对象已变化');
  return structuredClone({ requestId, target, pluginId: grant.pluginId, pluginRevision: grant.revision, contributionId: grant.contributionId, action });
}
