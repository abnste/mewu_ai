// SPDX-License-Identifier: MPL-2.0
import type { CaptureShortcutState } from './shortcut-contracts';
export interface ShortcutPortalView { requestId?: string; canceling: boolean; configuring: boolean }
/** Portal labels are display-only evidence; never turn a preferred chord into an active binding. */
export function portalShortcutLabel(state: CaptureShortcutState | undefined): string | undefined {
  return state?.backend === 'wayland_portal' && state.status === 'active' && state.actualTriggerDescription?.trim() ? state.actualTriggerDescription : undefined;
}
export function portalShortcutHasDisableAction(state: CaptureShortcutState | undefined): boolean {
  return state?.backend === 'wayland_portal' && state.status !== 'disabled' && (state.portalEnabled === true || state.status === 'active' || state.status === 'cleanup_pending');
}
interface Request { id: string; valid: boolean; cancellation?: Promise<void> }
/** Cancel uses its own RPC, even while configure awaits an OS consent dialog. */
export class ShortcutPortalController {
  private request?: Request;
  private disposed = false;
  constructor(private hooks: {
    allowed: () => boolean;
    configure: (expectedRevision: number, requestId: string) => Promise<CaptureShortcutState>;
    cancel: (requestId: string) => Promise<void>;
    accepted: (state: CaptureShortcutState) => void;
    changed: (view: ShortcutPortalView) => void;
    error: (error: unknown) => void;
    id?: () => string;
  }) {}
  private publish() { if (!this.disposed) this.hooks.changed({ requestId: this.request?.id, canceling: Boolean(this.request?.cancellation), configuring: Boolean(this.request?.valid) }); }
  async configure(expectedRevision: number): Promise<void> {
    if (this.disposed || this.request || !this.hooks.allowed()) return;
    const request: Request = { id: this.hooks.id?.() ?? crypto.randomUUID(), valid: true };
    this.request = request; this.publish();
    try {
      const state = await this.hooks.configure(expectedRevision, request.id);
      if (!this.disposed && this.request === request && request.valid) this.hooks.accepted(state);
    } catch (error) { if (!this.disposed && this.request === request && request.valid) this.hooks.error(error); }
    finally {
      if (this.request === request && request.valid) { this.request = undefined; this.publish(); }
    }
  }
  cancel(): Promise<void> {
    const request = this.request;
    if (!request) return Promise.resolve();
    request.valid = false;
    if (request.cancellation) return request.cancellation;
    const cancellation = Promise.resolve().then(() => this.hooks.cancel(request.id)).then(() => {
      if (this.request === request) { this.request = undefined; this.publish(); }
    }).finally(() => {
      if (this.request === request) { request.cancellation = undefined; this.publish(); }
    });
    request.cancellation = cancellation; this.publish();
    return cancellation;
  }
  cancelPublished(requestId: string): Promise<void> {
    if (this.disposed) return Promise.resolve();
    // Only adopt a nonce explicitly published by the native settings state.
    // A locally owned operation always wins over a possibly stale state event.
    if (!this.request) { this.request = { id: requestId, valid: false }; this.publish(); }
    return this.cancel();
  }
  dispose() { if (this.disposed) return; this.disposed = true; void this.cancel().catch(() => {}); }
}
