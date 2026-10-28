// SPDX-License-Identifier: MPL-2.0
import type { RunEvent, Scene } from './contracts';

export interface LiveToolOutput {callId:string;tool:string;text:string}
export interface LiveRunText { runId: string; text: string; reasoning: string;toolOutputs?:LiveToolOutput[] }
type RunView = Pick<Scene, 'id' | 'closed' | 'run' | 'messages'>;
interface Pending { runId: string; text: string[]; reasoning: string[];tools:Map<string,LiveToolOutput> }
function outputTail(text:string){const start=Math.max(0,text.length-16384);return text.slice(start+(start>0&&text.charCodeAt(start)>=0xdc00&&text.charCodeAt(start)<=0xdfff?1:0));}

/** Batch display updates only. Native persistence and final replies stay authoritative. */
export class RunStreamBuffer {
  private values: Record<string, LiveRunText> = {};
  private pending = new Map<string, Pending>();
  private timer?: ReturnType<typeof setTimeout>;
  private disposed = false;
  constructor(private target: (id: string) => RunView | undefined,
    private changed: (values: Record<string, LiveRunText>) => void) {}

  push(event: RunEvent) {
    const scene = this.target(event.sceneId);
    if (this.disposed || !scene || scene.closed || scene.run?.id !== event.runId || scene.run.status !== 'running' || (!event.text && !event.reasoning&&!event.toolOutput)) return;
    let value = this.pending.get(event.sceneId);
    if (!value || value.runId !== event.runId) {
      value = { runId: event.runId, text: [], reasoning: [],tools:new Map() }; this.pending.set(event.sceneId, value);
    }
    if (event.text) value.text.push(event.text);
    if (event.reasoning) value.reasoning.push(event.reasoning);
    const output=event.toolOutput;
    if(output&&typeof output.callId==='string'&&output.callId.length<=256&&typeof output.tool==='string'&&output.tool.length<=128&&typeof output.text==='string'){
      const previous=value.tools.get(output.callId);value.tools.set(output.callId,{callId:output.callId,tool:output.tool,text:outputTail((previous?.text??'')+outputTail(output.text))});
      if(value.tools.size>4)value.tools.delete(value.tools.keys().next().value!);
    }
    // Timers also drain when a WebView is hidden; animation frames may stop.
    this.timer ??= setTimeout(() => this.flush(), 40);
  }

  flush() {
    clearTimeout(this.timer); this.timer = undefined;
    if (this.disposed || !this.pending.size) return;
    const next = { ...this.values }; let changed = false;
    for (const [id, pending] of this.pending) {
      const scene = this.target(id);
      if (!scene || scene.closed || scene.run?.id !== pending.runId || scene.run.status !== 'running') continue;
      const before = next[id]?.runId === pending.runId ? next[id] : undefined;
      const tools=new Map((before?.toolOutputs??[]).map(tool=>[tool.callId,tool]));
      for(const tool of pending.tools.values()){const previous=tools.get(tool.callId);tools.delete(tool.callId);tools.set(tool.callId,{...tool,text:outputTail((previous?.text??'')+tool.text)});}
      while(tools.size>4)tools.delete(tools.keys().next().value!);
      next[id] = { runId: pending.runId, text: (before?.text ?? '') + pending.text.join(''), reasoning: (before?.reasoning ?? '') + pending.reasoning.join(''),...(tools.size?{toolOutputs:[...tools.values()]}:{}) }; changed = true;
    }
    this.pending.clear();
    if (changed) { this.values = next; this.changed(next); }
  }

  reconcile(scenes: RunView[]) {
    // Call before accepting a snapshot so cancellation keeps the last partial
    // reply, even if its last fragments were waiting for a display timer.
    this.flush();
    const byId = new Map(scenes.map(scene => [scene.id, scene]));
    const remove = Object.keys(this.values).filter(id => {
      const scene = byId.get(id), runId = this.values[id].runId;
      return !scene || scene.closed || scene.run?.id !== runId || scene.messages.some(message => message.role === 'assistant' && message.runId === runId);
    });
    if (!remove.length) return;
    const next = { ...this.values }; for (const id of remove) delete next[id];
    this.values = next; this.changed(next);
  }

  clear(id: string) {
    this.pending.delete(id);
    if (!this.pending.size) { clearTimeout(this.timer); this.timer = undefined; }
    if (!(id in this.values)) return;
    const next = { ...this.values }; delete next[id]; this.values = next;
    if (!this.disposed) this.changed(next);
  }

  dispose() { this.disposed = true; clearTimeout(this.timer); this.timer = undefined; this.pending.clear(); this.values = {}; }
}
