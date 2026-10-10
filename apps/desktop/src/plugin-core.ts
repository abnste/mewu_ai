// SPDX-License-Identifier: MPL-2.0
/** Historical registry records stay intact; these capabilities now belong to the host. */
const coreReplacements = new Set(['mewu.annotations', 'mewu.drawing', 'mewu.recording', 'mewu.recording-audio', 'mewu.video-trim', 'mewu.gif']);
export function isCoreReplacementPlugin(id: string): boolean { return coreReplacements.has(id); }
