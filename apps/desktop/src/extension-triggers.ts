// SPDX-License-Identifier: MPL-2.0
/** Hold-type modifier triggers for enabled plugins. The space window owns the
 * keyboard listeners; events travel to plugins as ordinary invocations carrying
 * only phase, pointer state and modifier flags — never typed characters. */
import type { ExtensionInvocation, ExtensionRecord, ExtensionService } from './extension-contracts';
import { triggerExtensionServices } from './extension-state';

export type TriggerCode = 'Alt' | 'Control' | 'Shift';
export type TriggerCombo = 'Ctrl' | 'Shift' | 'Alt';
export type TriggerModifier = 'ctrl' | 'shift' | 'alt';
export interface TriggerDescriptor { id: string; code: TriggerCode; with?: TriggerCombo[] }
export interface NormalizedTriggerDescriptor extends TriggerDescriptor { mask: number }
export type TriggerPhase = 'down' | 'up' | 'cancel';
export type TriggerCancelReason = 'escape' | 'blur' | 'modifiers_changed' | 'scene_changed';
export interface TriggerPointer { x: number; y: number; buttons: number }
export interface TriggerModifiers { ctrl: boolean; shift: boolean; alt: boolean }
export interface TriggerInput {
  type: 'trigger'; phase: TriggerPhase; reason?: TriggerCancelReason;
  pointer: TriggerPointer; modifiers: TriggerModifiers;
}

const MASK: Record<TriggerModifier, number> = { ctrl: 1, shift: 2, alt: 4 };
const MODIFIER_KEYS: Record<string, TriggerModifier> = { Alt: 'alt', Control: 'ctrl', Shift: 'shift' };
export const noModifiers: TriggerModifiers = { ctrl: false, shift: false, alt: false };

const codeModifier = (value: unknown): TriggerModifier | undefined =>
  value === 'Alt' ? 'alt' : value === 'Control' ? 'ctrl' : value === 'Shift' ? 'shift' : undefined;
const comboModifier = (value: unknown): TriggerModifier | undefined =>
  value === 'Alt' ? 'alt' : value === 'Ctrl' ? 'ctrl' : value === 'Shift' ? 'shift' : undefined;
export const modifierMask = (modifiers: TriggerModifiers): number =>
  (modifiers.ctrl ? MASK.ctrl : 0) | (modifiers.shift ? MASK.shift : 0) | (modifiers.alt ? MASK.alt : 0);

/** Registration metadata is untrusted; a service without a valid modifier
 * descriptor is ignored rather than armed. Registration-time rejection of
 * non-modifier keys happens in the extension runtime. */
export function normalizeTriggerDescriptor(service: ExtensionService): NormalizedTriggerDescriptor | undefined {
  const metadata = service.metadata;
  if (!metadata || typeof metadata !== 'object' || Array.isArray(metadata)) return;
  const trigger = (metadata as Record<string, unknown>).trigger;
  if (!trigger || typeof trigger !== 'object' || Array.isArray(trigger)) return;
  const fields = trigger as Record<string, unknown>;
  const code = codeModifier(fields.code);
  if (!code) return;
  const descriptor: NormalizedTriggerDescriptor = { id: service.id, code: fields.code as TriggerCode, mask: MASK[code] };
  if (fields.with === undefined) return descriptor;
  if (!Array.isArray(fields.with)) return;
  const extras: TriggerModifier[] = [];
  for (const value of fields.with) {
    const modifier = comboModifier(value);
    // A combination repeating the primary key or itself would never match.
    if (!modifier || modifier === code || extras.includes(modifier)) return;
    extras.push(modifier);
    descriptor.mask |= MASK[modifier];
  }
  // An empty combination array is equivalent to no combination.
  return extras.length ? { ...descriptor, with: fields.with as TriggerCombo[] } : descriptor;
}

/** Exact set equality: a bare Alt trigger must not fire while Ctrl+Alt is held,
 * and a Ctrl+Alt trigger must not fire while only Alt is held. */
export function triggerModifiersMatch(descriptor: NormalizedTriggerDescriptor, modifiers: TriggerModifiers): boolean {
  return modifierMask(modifiers) === descriptor.mask;
}

export function triggerInput(phase: TriggerPhase, modifiers: TriggerModifiers, pointer: TriggerPointer, reason?: TriggerCancelReason): TriggerInput {
  return { type: 'trigger', phase, ...(reason ? { reason } : {}), pointer, modifiers };
}

const eventModifiers = (event: KeyboardEvent): TriggerModifiers =>
  ({ ctrl: event.ctrlKey, shift: event.shiftKey, alt: event.altKey });
const keyupModifiers = (event: KeyboardEvent, released: TriggerModifier): TriggerModifiers => ({
  ctrl: released === 'ctrl' ? false : event.ctrlKey,
  shift: released === 'shift' ? false : event.shiftKey,
  alt: released === 'alt' ? false : event.altKey,
});

interface ArmedTrigger { pluginId: string; revision: number; serviceId: string }
interface TriggerSession { triggers: ArmedTrigger[]; mask: number; sceneId: string }

export interface ExtensionTriggerDeps {
  sceneId: () => string | undefined;
  records: () => ExtensionRecord[];
  invoke: (invocation: ExtensionInvocation) => Promise<unknown>;
}

/** Install in the space window only. Returns the detach function; a hold that
 * is still armed at teardown ends without a plugin-visible event. */
export function attachExtensionTriggers(deps: ExtensionTriggerDeps): (() => void) & { reconcile(): void } {
  let session: TriggerSession | undefined;
  let pointer: TriggerPointer = { x: 0, y: 0, buttons: 0 };
  const armedTriggers = (modifiers: TriggerModifiers): ArmedTrigger[] => {
    if (!deps.sceneId()) return [];
    return triggerExtensionServices(deps.records()).flatMap(({ record, service }) => {
      const descriptor = normalizeTriggerDescriptor(service);
      return descriptor && triggerModifiersMatch(descriptor, modifiers)
        ? [{ pluginId: record.manifest.id, revision: record.revision, serviceId: service.id }]
        : [];
    });
  };
  // Preserve the original scene and registration revision across the hold.
  // A scene switch must never turn its release into a commit in the next scene.
  const dispatch = (held: TriggerSession, phase: TriggerPhase, modifiers: TriggerModifiers, reason?: TriggerCancelReason) => {
    for (const trigger of held.triggers) {
      const input = triggerInput(phase, { ...modifiers }, { ...pointer }, reason);
      try {
        void deps.invoke({ ...trigger, input, sceneId: held.sceneId }).catch(error => console.error(error));
      } catch (error) { console.error(error); }
    }
  };
  const cancel = (reason: TriggerCancelReason, modifiers = noModifiers) => {
    if (!session) return;
    const held = session; session = undefined;
    dispatch(held, 'cancel', modifiers, reason);
  };
  const reconcile = () => {
    if (!session) return;
    if (deps.sceneId() !== session.sceneId) { cancel('scene_changed'); return; }
    const current = triggerExtensionServices(deps.records());
    const surviving = session.triggers.filter(trigger => current.some(({record,service}) =>
      record.manifest.id === trigger.pluginId && record.revision === trigger.revision && service.id === trigger.serviceId));
    // Revoked registrations cannot be invoked again, even to deliver cancel.
    if (!surviving.length) session = undefined;
    else session = { ...session, triggers: surviving };
  };
  const keydown = (event: KeyboardEvent) => {
    reconcile();
    if (event.isComposing || event.keyCode === 229) return;
    const modifiers = eventModifiers(event);
    if (session && event.key === 'Escape') {
      event.preventDefault(); event.stopPropagation();
      cancel('escape', modifiers); return;
    }
    if (session && (event.metaKey || modifierMask(modifiers) !== session.mask)) cancel('modifiers_changed', modifiers);
    if (session || event.repeat || event.metaKey || !MODIFIER_KEYS[event.key]) return;
    const sceneId = deps.sceneId();
    const triggers = armedTriggers(modifiers);
    if (!sceneId || !triggers.length) return;
    session = { triggers, mask: modifierMask(modifiers), sceneId };
    dispatch(session, 'down', modifiers);
  };
  const keyup = (event: KeyboardEvent) => {
    reconcile();
    if (!session) return;
    const released = MODIFIER_KEYS[event.key];
    if (!released || !(session.mask & MASK[released])) return;
    const held = session; session = undefined;
    const modifiers = keyupModifiers(event, released);
    // Extra modifiers reported only on release invalidate the hold as well.
    if (event.metaKey || (modifierMask(modifiers) & ~held.mask)) dispatch(held, 'cancel', modifiers, 'modifiers_changed');
    else dispatch(held, 'up', modifiers);
  };
  const blur = () => cancel('blur');
  const pointermove = (event: PointerEvent) => {
    pointer = { x: event.clientX, y: event.clientY, buttons: event.buttons };
  };
  window.addEventListener('keydown', keydown, true);
  window.addEventListener('keyup', keyup, true);
  window.addEventListener('blur', blur);
  window.addEventListener('pointermove', pointermove, { capture: true, passive: true });
  const detach = () => {
    session = undefined;
    window.removeEventListener('keydown', keydown, true);
    window.removeEventListener('keyup', keyup, true);
    window.removeEventListener('blur', blur);
    window.removeEventListener('pointermove', pointermove, { capture: true });
  };
  return Object.assign(detach, { reconcile });
}
