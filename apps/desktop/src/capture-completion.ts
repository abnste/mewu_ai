// SPDX-License-Identifier: MPL-2.0
export async function finishCapture(actions: { prepare: () => Promise<void>; current: () => boolean; export: () => Promise<boolean | void>; close?: () => Promise<boolean | void> }) {
  await actions.prepare();
  if (!actions.current()) return false;
  if (await actions.export() === false) return false;
  if (!actions.current()) return false;
  if (await actions.close?.() === false) return false;
  return true;
}
export async function finishDrawing(actions: { saveText: () => Promise<boolean>; copy: () => Promise<boolean>; current: () => boolean; close: () => void }) {
  if (!await outputDrawing({ saveText: actions.saveText, output: actions.copy, current: actions.current })) return false;
  actions.close(); return true;
}
export async function outputDrawing(actions: { saveText: () => Promise<boolean>; output: () => Promise<boolean>; current: () => boolean }) {
  if (!await actions.saveText() || !actions.current()) return false;
  return await actions.output() && actions.current();
}
