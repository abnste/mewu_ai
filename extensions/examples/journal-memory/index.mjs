// SPDX-License-Identifier: MPL-2.0
import { definePlugin } from '@mewu/sdk';

function scope(context) {
  if (!context.agentId || context.agentId.length > 128) throw new Error('Agent context is required');
  return `agent.${encodeURIComponent(context.agentId)}.`;
}

export default definePlugin(ctx => {
  ctx.memory.register({
    id: 'example.journal-memory.provider', title: '日志记忆',
    description: '独立记录持久存储，不限制为固定条数。',
    metadata: { operations: ['retain', 'recall', 'forget'] },
    inputSchema: { type: 'object', properties: {
      operation: { enum: ['retain', 'recall', 'forget'] },
      text: { type: 'string', maxLength: 65536 }, query: { type: 'string', maxLength: 4000 },
      id: { type: 'string' }, cursor: { type: 'string' }, limit: { type: 'integer', minimum: 1, maximum: 100 }
    }, required: ['operation'], additionalProperties: false }
  }, async (input, { context }) => {
    const prefix = scope(context);
    if (input.operation === 'retain') {
      const text = String(input.text || '').trim();
      if (!text || text.length > 65536) throw new Error('Invalid memory text');
      const sequenceKey = `${prefix}sequence`;
      let sequence;
      let reserved = false;
      for (let attempt = 0; attempt < 32; attempt++) {
        const previous = await ctx.storage.get(sequenceKey);
        sequence = previous === null ? 1 : previous + 1;
        if (!Number.isSafeInteger(sequence) || sequence < 1) throw new Error('Memory sequence exhausted');
        if (await ctx.storage.compareAndSet(sequenceKey, previous, sequence)) { reserved = true; break; }
      }
      if (!reserved) throw new Error('Memory is busy; retry the retain request');
      // Reserving an ID is atomic across concurrent runs of this provider.
      // Interrupted writes may leave a gap, never overwrite an existing record.
      const id = `${prefix}entry.${String(sequence).padStart(16, '0')}`;
      await ctx.storage.set(id, { id, text, createdAt: new Date().toISOString() });
      return { id, retained: true };
    }
    if (input.operation === 'forget') {
      if (typeof input.id !== 'string' || !input.id.startsWith(`${prefix}entry.`)) throw new Error('Memory belongs to another agent');
      await ctx.storage.delete(input.id);
      return { forgotten: true };
    }
    if (input.operation !== 'recall') throw new Error('Unknown memory operation');
    const query = String(input.query || '').trim().toLocaleLowerCase();
    const limit = input.limit === undefined ? 20 : input.limit;
    if (!Number.isInteger(limit) || limit < 1 || limit > 100) throw new Error('Invalid recall page size');
    const terms = query.split(/\s+/u).filter(Boolean);
    const page = await ctx.storage.list({ prefix: `${prefix}entry.`, cursor: input.cursor || '' });
    const matches = [];
    let nextCursor = page.nextCursor;
    for (let index = 0; index < page.keys.length; index++) {
      const key = page.keys[index];
      if (!key.startsWith(`${prefix}entry.`)) continue;
      const record = await ctx.storage.get(key);
      if (!record || typeof record.text !== 'string') continue;
      const text = record.text.toLocaleLowerCase();
      const score = terms.length ? terms.filter(term => text.includes(term)).length : 1;
      if (score > 0) matches.push({ ...record, score });
      if (matches.length === limit) {
        // A host page can contain more matching records than the requested
        // result limit. Resume AFTER the last consumed key, not the host page's
        // last key, or an entire group of memories would become unreachable.
        nextCursor = index + 1 < page.keys.length || page.nextCursor !== null ? key : null;
        break;
      }
    }
    matches.sort((a, b) => b.score - a.score || b.id.localeCompare(a.id));
    return { memories: matches, nextCursor, exhaustive: nextCursor === null };
  });
});
