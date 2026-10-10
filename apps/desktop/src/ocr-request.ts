// SPDX-License-Identifier: MPL-2.0
import type { OcrTarget } from './contracts';

export interface OcrRequest { requestId: string; pluginId: string; revision: number; contributionId: string; target: OcrTarget }

// A canceled invocation can still settle. It must not clear a newer operation or publish UI state.
export class OcrRequestGate<T> {
  current?: OcrRequest;
  private disposed = false;
  constructor(private hooks: {
    run: (request: OcrRequest) => Promise<T>;
    cancel: (requestId: string) => Promise<void>;
    pending: (request: OcrRequest | undefined) => void;
    result: (value: T, request: OcrRequest) => void;
    error: (message: string, request: OcrRequest) => void;
  }) {}
  async start(request: OcrRequest): Promise<void> {
    if (this.disposed) return;
    this.cancel(); this.current = request; this.hooks.pending(request);
    try {
      const value = await this.hooks.run(request);
      if (!this.disposed && this.current === request) this.hooks.result(value, request);
    } catch (error) {
      if (!this.disposed && this.current === request) this.hooks.error(error instanceof Error ? error.message : String(error), request);
    } finally {
      if (!this.disposed && this.current === request) { this.current = undefined; this.hooks.pending(undefined); }
    }
  }
  cancel() {
    const request = this.current;
    if (!request) return;
    this.current = undefined; this.hooks.pending(undefined);
    void this.hooks.cancel(request.requestId).catch(() => undefined);
  }
  dispose() { this.cancel(); this.disposed = true; }
}
