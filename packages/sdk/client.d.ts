// SPDX-License-Identifier: MPL-2.0
export interface ClientSurface {
  readonly ready: Promise<void>;
  call<T = unknown>(input: unknown): Promise<T>;
  setDropTargets(targets: Array<{id:string;x:number;y:number;width:number;height:number}>): Promise<void>;
  on(name: string, callback: (value: unknown) => void): () => void;
  viewportPoint(x:number,y:number): Promise<{x:number;y:number}>;
  /** Consume a real pointerdown once. Coordinates on release are client-local.
   * Escape, close, focus loss and a 30 s deadline cancel the gesture. */
  drag(event: PointerEvent): Promise<{cancelled:boolean;x:number;y:number}>;
  dispose(): void;
}
declare global { const mewu: ClientSurface; }
