// SPDX-License-Identifier: MPL-2.0
export interface ClientSurface {
  readonly ready: Promise<void>;
  /** Render an authorized object document in a disposable, opaque offline frame. */
  previewDocument(input:{html:string;width?:number;height?:number}):Promise<string|null>;
  call<T = unknown>(input: unknown): Promise<T>;
  /** One optional default target receives a drop on this surface's launcher. */
  setDropTargets(targets: Array<{id:string;x:number;y:number;width:number;height:number;default?:boolean}>): Promise<void>;
  on(name: string, callback: (value: unknown) => void): () => void;
  /** Flush edits before the host closes this surface. Reject to keep it open. */
  onBeforeClose(callback: () => void | Promise<void>): () => void;
  viewportPoint(x:number,y:number): Promise<{x:number;y:number}>;
  /** Consume a real pointerdown once. Coordinates on release are client-local.
   * Escape, close, focus loss and a 30 s deadline cancel the gesture. */
  drag(event: PointerEvent, preview?: {image?:string;label?:string}): Promise<{cancelled:boolean;x:number;y:number}>;
  dispose(): void;
}
declare global { const mewu: ClientSurface; }
