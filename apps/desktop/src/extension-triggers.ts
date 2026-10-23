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
export type TriggerCancelReason = 'escape' | 'blur';
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
interface TriggerSession { triggers: ArmedTrigger[]; mask: number }

export interface ExtensionTriggerDeps {
  sceneId: () => string | undefined;
  records: () => ExtensionRecord[];
  invoke: (invocation: ExtensionInvocation) => Promise<unknown>;
}

/** Install in the space window only. Returns the detach function; a hold that
 * is still armed at teardown ends without a plugin-visible event. */
export function attachExtensionTriggers(deps: ExtensionTriggerDeps): () => void {
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
  // Fire-and-forget: one plugin's failure must not affect other listeners.
  const dispatch = (triggers: ArmedTrigger[], phase: TriggerPhase, modifiers: TriggerModifiers, reason?: TriggerCancelReason) => {
    const sceneId = deps.sceneId();
    if (!sceneId) return;
    const input = triggerInput(phase, modifiers, pointer, reason);
    for (const trigger of triggers) {
      deps.invoke({ pluginId: trigger.pluginId, revision: trigger.revision, serviceId: trigger.serviceId, input, sceneId })
        .catch(error => console.error(error));
    }
  };
  const keydown = (event: KeyboardEvent) => {
    if (event.isComposing) return;
    if (session) {
      // While armed the hold owns Escape; other keys keep the session as is.
      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopPropagation();
        dispatch(session.triggers, 'cancel', eventModifiers(event), 'escape');
        session = undefined;
      }
      return;
    }
    // Auto-repeat must not re-enter the down phase. Bare modifiers do not
    // type, so unlike text shortcuts they may arm while an input is focused.
    if (event.repeat || !MODIFIER_KEYS[event.key]) return;
    const modifiers = eventModifiers(event);
    const triggers = armedTriggers(modifiers);
    if (!triggers.length) return;
    session = { triggers, mask: modifierMask(modifiers) };
    dispatch(triggers, 'down', modifiers);
  };
  const keyup = (event: KeyboardEvent) => {
    if (!session) return;
    const released = MODIFIER_KEYS[event.key];
    if (!released || !(session.mask & MASK[released])) return;
    dispatch(session.triggers, 'up', keyupModifiers(event, released));
    session = undefined;
  };
  // Losing the window invalidates every held modifier at once.
  const blur = () => {
    if (!session) return;
    dispatch(session.triggers, 'cancel', noModifiers, 'blur');
    session = undefined;
  };
  const pointermove = (event: PointerEvent) => {
    pointer = { x: event.clientX, y: event.clientY, buttons: event.buttons };
  };
  window.addEventListener('keydown', keydown, true);
  window.addEventListener('keyup', keyup, true);
  window.addEventListener('blur', blur);
  window.addEventListener('pointermove', pointermove, { capture: true, passive: true });
  return () => {
    session = undefined;
    window.removeEventListener('keydown', keydown, true);
    window.removeEventListener('keyup', keyup, true);
    window.removeEventListener('blur', blur);
    window.removeEventListener('pointermove', pointermove, { capture: true });
  };
}
