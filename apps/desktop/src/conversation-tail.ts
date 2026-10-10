// SPDX-License-Identifier: MPL-2.0
/** Follow settled content/image layout only while the reader is at the tail. */
export function observeConversationTail(element: HTMLElement, follow: () => boolean): () => void {
  let disposed = false, queued = false;
  const schedule = () => {
    if (disposed || queued) return;
    queued = true;
    queueMicrotask(() => { queued = false; if (!disposed && follow()) element.scrollTop = element.scrollHeight; });
  };
  const resize = new ResizeObserver(schedule);
  const watch = () => { resize.disconnect(); resize.observe(element); for (const child of element.children) resize.observe(child); schedule(); };
  const changes = new MutationObserver(watch);
  changes.observe(element, { childList: true }); watch();
  return () => { disposed = true; changes.disconnect(); resize.disconnect(); };
}
