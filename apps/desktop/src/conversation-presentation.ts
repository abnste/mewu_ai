// SPDX-License-Identifier: MPL-2.0
import type { Message, Scene } from './contracts';

/** Compact view shows the current turn, never a question from an older run. */
export function currentTurnPrompt(messages: Message[], runId?: string): Message | undefined {
  for (let index = messages.length - 1; index >= 0; index--) {
    const message = messages[index];
    if (message.role === 'user' && (!runId || message.runId === runId)) return message;
  }
}

export function thinkingGlowVisible(scene: Pick<Scene, 'run' | 'closed' | 'frozen'>, closing: boolean, enabled = true): boolean {
  return enabled && !closing && !scene.closed && !scene.frozen && scene.run?.status === 'running';
}

/** Only a color can reach the CSS custom property; no CSS expression is accepted. */
export function thinkingGlowColor(value?: string): string {
  return value && /^#[0-9a-f]{6}$/i.test(value) ? value.toUpperCase() : '#A7C7FF';
}
