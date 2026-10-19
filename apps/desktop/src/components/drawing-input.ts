// SPDX-License-Identifier: MPL-2.0
/** Pair physical mouse presses without assuming MouseEvent.buttons is an event-time snapshot. */
export class DrawingMouseInput {
  private device = '';
  private mouseId: number | undefined;
  private handled = false;
  private handledId: number | undefined;
  private fallback = new WeakSet<Event>();
  private cancelMouse: (() => void) | undefined;

  observe(event: PointerEvent): void {
    this.device = event.pointerType;
    if (event.pointerType === 'mouse' && event.isPrimary !== false) this.mouseId = event.pointerId;
    else if (event.pointerType !== 'mouse') this.cancelMouse?.();
  }
  pointerDown(event: PointerEvent): boolean {
    this.observe(event);
    // A late pointer stream must not start a second copy of an accepted mouse gesture.
    if (this.cancelMouse) return false;
    this.handled = true; this.handledId = event.pointerId;
    return true;
  }
  mouseDown(event: MouseEvent): PointerEvent | undefined {
    if (this.handled || this.cancelMouse || this.device !== 'mouse' || this.mouseId === undefined
      || event.button !== 0 || event.buttons !== 1 || this.fromTouch(event)) return;
    this.handled = true; this.handledId = this.mouseId;
    const normalized = this.sample(event, this.mouseId, 'pointerdown');
    this.fallback.add(normalized);
    return normalized;
  }
  private fromTouch(event: MouseEvent): boolean {
    return (event as MouseEvent & { sourceCapabilities?: { firesTouchEvents?: boolean } }).sourceCapabilities?.firesTouchEvents === true;
  }
  private sample(event: MouseEvent, pointerId: number, type: string): PointerEvent {
    const buttons = type === 'pointerup' ? 0 : event.buttons;
    return { type, pointerId, pointerType: 'mouse', isPrimary: true, button: event.button, buttons,
      clientX: event.clientX, clientY: event.clientY, shiftKey: event.shiftKey, ctrlKey: event.ctrlKey,
      altKey: event.altKey, metaKey: event.metaKey, pressure: buttons & 1 ? .5 : 0,
      target: event.target, currentTarget: event.currentTarget, timeStamp: event.timeStamp,
      preventDefault: () => event.preventDefault(), stopPropagation: () => event.stopPropagation(),
      stopImmediatePropagation: () => event.stopImmediatePropagation() } as unknown as PointerEvent;
  }
  attach(document: Document): () => void {
    const down = (event: Event) => this.observe(event as PointerEvent);
    const end = (event: Event) => {
      const pointer = event as PointerEvent;
      if (pointer.pointerId === this.handledId) { this.handled = false; this.handledId = undefined; }
    };
    // The event type/button is authoritative even when a queued mouseup still says buttons=1.
    const up = (event: Event) => { if ((event as MouseEvent).button === 0) { this.handled = false; this.handledId = undefined; } };
    document.addEventListener('pointerdown', down, true);
    document.addEventListener('pointerup', end, true);
    document.addEventListener('pointercancel', end, true);
    document.addEventListener('mouseup', up, true);
    return () => {
      document.removeEventListener('pointerdown', down, true);
      document.removeEventListener('pointerup', end, true);
      document.removeEventListener('pointercancel', end, true);
      document.removeEventListener('mouseup', up, true);
      this.reset();
    };
  }
  capture(svg: SVGSVGElement, event: PointerEvent): void {
    if (!this.fallback.has(event)) svg.setPointerCapture(event.pointerId);
  }
  release(svg: SVGSVGElement, event: PointerEvent): void {
    if (!this.fallback.has(event) && svg.hasPointerCapture(event.pointerId)) svg.releasePointerCapture(event.pointerId);
  }
  listen(window: Window, svg: SVGSVGElement, event: PointerEvent,
    moved: (event: PointerEvent) => void, released: (event: PointerEvent) => void,
    canceled: (event: PointerEvent) => void, cancel: () => void): () => void {
    const mouse = this.fallback.has(event);
    const move = mouse ? (next: Event) => {
      const value = next as MouseEvent;
      if (this.fromTouch(value) || value.buttons !== 0 && value.buttons !== 1) { cancel(); return; }
      moved(this.sample(value, event.pointerId, 'pointermove'));
    } : moved as EventListener;
    const up = mouse ? (next: Event) => {
      const value = next as MouseEvent;
      if (value.button === 0 && !this.fromTouch(value)) released(this.sample(value, event.pointerId, 'pointerup'));
    } : released as EventListener;
    if (mouse) this.cancelMouse = cancel;
    window.addEventListener(mouse ? 'mousemove' : 'pointermove', move);
    window.addEventListener(mouse ? 'mouseup' : 'pointerup', up);
    window.addEventListener('pointercancel', canceled as EventListener);
    svg.addEventListener('lostpointercapture', canceled as EventListener);
    return () => {
      window.removeEventListener(mouse ? 'mousemove' : 'pointermove', move);
      window.removeEventListener(mouse ? 'mouseup' : 'pointerup', up);
      window.removeEventListener('pointercancel', canceled as EventListener);
      svg.removeEventListener('lostpointercapture', canceled as EventListener);
      if (this.cancelMouse === cancel) this.cancelMouse = undefined;
    };
  }
  reset(): void {
    this.cancelMouse?.();
    this.handled = false; this.handledId = undefined; this.device = ''; this.mouseId = undefined;
  }
}
