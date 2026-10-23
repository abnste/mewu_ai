// SPDX-License-Identifier: MPL-2.0
// Host action facade for executable extensions. Native code owns permission
// dispatch and the pending-reply channel; the space window owns each action.
export type HostActionName = 'copy' | 'save' | 'pin' | 'reference';
export type HostActionVariant = 'annotated' | 'original';
export interface HostActionRequest { action: HostActionName; regionId: string; variant?: HostActionVariant }
export type HostActionResult = { ok: true } | { ok: false; reason: string };
export interface ExtensionActionRequest extends HostActionRequest { id: string }

const actionNames: readonly HostActionName[] = ['copy', 'save', 'pin', 'reference'];
const variants: readonly HostActionVariant[] = ['annotated', 'original'];
const fields = new Set(['id', 'action', 'regionId', 'variant']);

function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}
function identifier(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= 256
    && !/[\u0000-\u001f\u007f]/u.test(value);
}
/** Native code validates before emitting; this second gate keeps the space
 * window authoritative over its own request shape. */
export function parseExtensionActionRequest(value: unknown): ExtensionActionRequest {
  if (!object(value) || Object.keys(value).some(key => !fields.has(key))
    || !identifier(value.id) || !identifier(value.regionId)) throw new Error('宿主动作请求无效');
  const action = value.action as HostActionName;
  if (!actionNames.includes(action)) throw new Error('宿主动作请求无效');
  if (value.variant !== undefined && value.variant !== null
    && !variants.includes(value.variant as HostActionVariant)) throw new Error('图片导出版本无效');
  const variant = value.variant == null ? undefined : value.variant as HostActionVariant;
  return { id: value.id, action, regionId: value.regionId, ...(variant ? { variant } : {}) };
}
/** Only bounded first-party text becomes an action reason; anything else falls
 * back to the fixed generic message instead of leaking foreign error bodies. */
export function extensionActionReason(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return message.length > 0 && message.length <= 256 && !/[\u0000-\u001f\u007f]/u.test(message)
    ? message : '宿主动作执行失败';
}

export interface ExtensionActionActions {
  execute(request: HostActionRequest): Promise<HostActionResult>;
  resolve(id: string, ok: boolean, reason?: string): Promise<void>;
}
export type ExtensionActionListen = (name: 'extension-action-request', handler: (event: { payload: unknown }) => void,
  options: { target: 'space' }) => Promise<() => void>;
/** Install only in the first-party space. A lost reply surfaces as the native
 * 30-second action timeout; the request is never replayed automatically. */
export async function attachExtensionActions(actions: ExtensionActionActions, injectedListen?: ExtensionActionListen): Promise<() => void> {
  const listen: ExtensionActionListen = injectedListen ?? (await import('@tauri-apps/api/event')).listen;
  let stop: (() => void) | undefined;
  try {
    stop = await listen('extension-action-request', event => {
      void (async () => {
        let id: string | undefined;
        try {
          const request = parseExtensionActionRequest(event.payload);
          id = request.id;
          const reply = await actions.execute(request);
          await actions.resolve(request.id, reply.ok, reply.ok ? undefined : reply.reason);
        } catch (error) {
          // A malformed request still answers with its id when one is readable.
          if (!id && object(event.payload) && identifier(event.payload.id)) id = event.payload.id;
          if (id) await actions.resolve(id, false, extensionActionReason(error)).catch(() => {});
        }
      })();
    }, { target: 'space' });
  } catch (error) { throw error; }
  return () => { stop?.(); stop = undefined; };
}
