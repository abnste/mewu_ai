// SPDX-License-Identifier: MPL-2.0
export type HookCategory = 'cron' | 'prompt' | 'reply' | 'shortcut' | 'event';
export interface HookMatcher { mode: 'contains' | 'exact' | 'regex'; value: string; ignoreCase: boolean }
export type HookTrigger =
  | { kind: 'cron'; expression: string; timezone: string }
  | { kind: 'prompt' | 'reply'; matcher: HookMatcher }
  | { kind: 'shortcut'; accelerator: string }
  | { kind: 'hold'; accelerator: string; milliseconds:number }
  | { kind: 'event'; name: string };
export type HookAction =
  | { kind: 'injectPrompt'; text: string }
  | { kind: 'sendPrompt'; text: string; sceneId?: string | null }
  | { kind: 'mcp'; serverId: string; tool: string; arguments: Record<string, unknown> }
  | { kind: 'command'; shell: 'cmd' | 'powershell' | 'sh'; code: string; cwd?: string | null }
  | { kind: 'program'; executable: string; arguments: string[]; cwd?: string | null }
  | { kind: 'openUrl'; url: string }
  | { kind: 'openApp'; path: string }
  | { kind: 'input'; steps: HookInput[] }
  | { kind: 'script'; code: string }
  | { kind: 'emit'; name: string; payload: unknown }
  | { kind: 'app'; method: string; params: Record<string, unknown> }
  | { kind: 'delay'; milliseconds: number };
export type HookInput = { kind: 'move'; x: number; y: number } | { kind: 'click'; button: 'left' | 'right' | 'middle'; count: number } | { kind:'button'; button:'left'|'right'|'middle'; direction:'press'|'release' } | { kind: 'key'; key: string } | { kind: 'text'; text: string } | { kind: 'scroll'; x: number; y: number } | { kind: 'delay'; milliseconds: number };
export interface HookRule { id: string; revision: number; name: string; enabled: boolean; trigger: HookTrigger; condition: string; actions: HookAction[]; cooldownMs: number; timeoutMs: number; stopOnError: boolean }
export function hookCategory(trigger:HookTrigger):HookCategory{return trigger.kind==='hold'?'shortcut':trigger.kind;}
export interface HookRun { id: string; ruleId: string; ruleRevision: number; eventId: string; eventName: string; startedAt: number; finishedAt?: number | null; status: 'running' | 'completed' | 'failed' | 'canceled' | 'skipped' | 'interrupted'; completedActions: number; error?: string | null }
export interface HookSnapshot { revision: number; rules: HookRule[]; runs: HookRun[]; enabled: boolean; shortcuts: Record<string, { status: string; message?: string; label?: string }>; nextRuns: Record<string, number>; events: { name: string; title: string }[]; mcpServers: { id: string; name: string; tools: { name: string }[] }[]; platform: string;warning?:string|null }
export function newHookRule(category: HookCategory, platform = ''): HookRule {
  const trigger: HookTrigger = category === 'cron' ? { kind: 'cron', expression: '0 9 * * *', timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || 'local' }
    : category === 'prompt' || category === 'reply' ? { kind: category, matcher: { mode: category === 'reply' ? 'regex' : 'contains', value: category === 'prompt' ? '？' : '.*', ignoreCase: false } }
    : category === 'shortcut' ? { kind: 'shortcut', accelerator: '' } : { kind: 'event', name: 'app.started' };
  return { id: crypto.randomUUID(), revision: 0, name: '', enabled: true, trigger, condition: '', actions: [newHookAction(category === 'prompt' ? 'injectPrompt' : 'sendPrompt', platform)], cooldownMs: 500, timeoutMs: 10000, stopOnError: true };
}
export function newHookAction(kind: HookAction['kind'], platform = ''): HookAction {
  switch (kind) {
    case 'injectPrompt': case 'sendPrompt': return { kind, text: '' };
    case 'mcp': return { kind, serverId: '', tool: '', arguments: {} };
    case 'command': return { kind, shell: platform === 'windows' ? 'powershell' : 'sh', code: '', cwd: null };
    case 'program': return { kind, executable: '', arguments: [], cwd: null };
    case 'openUrl': return { kind, url: '' };
    case 'openApp': return { kind, path: '' };
    case 'input': return { kind, steps: [{ kind: 'key', key: platform === 'macos' ? 'Super+C' : 'Control+C' }] };
    case 'script': return { kind, code: 'return { actions: [{ kind: "delay", milliseconds: 0 }] };' };
    case 'emit': return { kind, name: 'custom.', payload: {} };
    case 'app': return { kind, method: 'capture.open', params: {} };
    case 'delay': return { kind, milliseconds: 500 };
  }
}
export function formatHookShortcut(value:string,platform=''):string {
  const names:Record<string,string>={control:'Ctrl',ctrl:'Ctrl',shift:'Shift',alt:platform==='macos'?'⌥':'Alt',metaleft:platform==='macos'?'⌘':platform==='windows'?'Win':'Super',super:platform==='macos'?'⌘':platform==='windows'?'Win':'Super',meta:platform==='macos'?'⌘':platform==='windows'?'Win':'Super'};
  return value.split('+').map(part=>{const key=part.trim();if(/^Key[A-Z]$/i.test(key))return key.slice(3).toUpperCase();if(/^Digit\d$/i.test(key))return key.slice(5);return names[key.toLowerCase()]||(/^[a-z]$/i.test(key)?key.toUpperCase():key);}).join(' + ');
}
export function hookAccelerator(event: Pick<KeyboardEvent, 'code' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey' | 'isComposing'> & Partial<Pick<KeyboardEvent, 'key'>>): string | undefined {
  let code = event.code;
  // Remote/accessibility input may have a logical key without a physical code.
  // Preserve known physical layouts; do not guess punctuation/dead-key positions.
  if (!code || code === 'Unidentified') {
    const key = event.key || '';
    code = /^[a-z]$/i.test(key) ? `Key${key.toUpperCase()}` : /^\d$/.test(key) ? `Digit${key}` : key === ' ' ? 'Space' : key;
  }
  if (event.isComposing || /^(Control|Shift|Alt|Meta)/.test(code)) return;
  if (!/^(Key[A-Z]|Digit\d|F(?:[1-9]|1\d|2[0-4])|Space|Enter|Tab|Escape|Backspace|Delete|Insert|Home|End|PageUp|PageDown|Arrow(?:Up|Down|Left|Right)|PrintScreen|Pause|Backquote|Minus|Equal|BracketLeft|BracketRight|Backslash|Semicolon|Quote|Comma|Period|Slash)$/.test(code)) return;
  if (!(event.ctrlKey || event.altKey || event.metaKey) && !/^F\d|PrintScreen|Pause$/.test(code)) return;
  return [event.ctrlKey && 'Control', event.altKey && 'Alt', event.shiftKey && 'Shift', event.metaKey && 'Super', code].filter(Boolean).join('+');
}
