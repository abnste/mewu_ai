// SPDX-License-Identifier: MPL-2.0
import type { AgentMemoryStatus, MemoryBindingView, MemoryEvidenceView, MemoryProviderSelection, MemoryReason, MemoryWriteReceipt } from './contracts';

export function normalizeMemoryEndpoint(value: string): string {
  let url: URL;
  try { url = new URL(value.trim()); } catch { throw new Error('请输入有效的服务地址'); }
  const local = url.hostname === 'localhost' || url.hostname === '[::1]' || /^127(?:\.\d{1,3}){3}$/.test(url.hostname);
  if (url.protocol !== 'https:' && !(url.protocol === 'http:' && local)) throw new Error('服务地址需使用 HTTPS，本机服务可用 HTTP');
  if (url.username || url.password || url.search || url.hash) throw new Error('服务地址不能包含账号、参数或片段');
  return url.href.replace(/\/+$/, '');
}
export const memoryBindingLabels: Record<MemoryBindingView['status'], string> = { provisioning: '正在配置', ready: '就绪', suspended: '已暂停', retired: '已停用' };
const evidenceLabels: Record<MemoryWriteReceipt['state'], string> = { queued: '等待同步', accepted: '远端处理中', committed: '已同步', blocked: '同步受阻', unknown: '结果待确认', failed: '同步失败', suppressed: '不再使用' };
const reasonLabels: Record<MemoryReason, string> = { queue_full: '待处理记录已满', evidence_too_large: '内容超过同步上限', authority_changed: '授权已变化', provider_unavailable: '服务不可用', provider_rejected: '服务拒绝', invalid_response: '服务响应无效', outcome_unknown: '结果待确认', cancelled_before_dispatch: '未上传', cleanup_needs_authorization: '删除需要原连接授权' };
export const memoryReasonLabel = (reason: MemoryReason | null) => reason ? reasonLabels[reason] ?? '处理受阻' : '';
export function memoryWriteLabel(value: Pick<MemoryWriteReceipt, 'state' | 'reason'>): string {
  return evidenceLabels[value.state] ?? '状态待确认';
}
export function memoryEvidenceLabel(value: MemoryEvidenceView): string {
  const forgotten = value.forgotten;
  if (!forgotten?.localSuppressed) return memoryWriteLabel(value);
  if (forgotten.remoteState === 'confirmed_deleted' && forgotten.quiescence === 'proven') return '不再使用 · 远端已删除';
  const labels = { pending: '远端待删除', currently_absent: '远端删除待确认', confirmed_deleted: '远端删除待确认', unknown: '远端删除待确认', failed: '远端删除失败', needs_authorization: '远端删除待授权' };
  return `不再使用 · ${labels[forgotten.remoteState] ?? '远端删除待确认'}`;
}
export function mergeMemoryStatus(previous: AgentMemoryStatus | undefined, incoming: AgentMemoryStatus): AgentMemoryStatus {
  if (!previous) return incoming;
  if (incoming.selection.revision < previous.selection.revision) return previous;
  return { ...incoming, bindings: incoming.bindings.map(value => {
    const old = previous.bindings.find(item => item.id === value.id);
    if (!old) return value;
    const authority = old.revision > value.revision ? old : value;
    const ledger = old.ledgerRevision > value.ledgerRevision ? old : value;
    return { ...authority, ledgerRevision: ledger.ledgerRevision, evidenceCount: ledger.evidenceCount, pendingCount: ledger.pendingCount, blockedCount: ledger.blockedCount, deletionPendingCount: ledger.deletionPendingCount };
  }) };
}
export function manualMemoryText(value: string): string {
  const text = value.trim();
  if (!text) throw new Error('请输入记录内容');
  if (text.includes('\0')) throw new Error('内容包含无效字符');
  if (new TextEncoder().encode(text).byteLength > 64 * 1024) throw new Error('单条记录最多 64 KiB');
  return text;
}
export function memorySelectionSettled(latest: MemoryProviderSelection | undefined, receipt: MemoryProviderSelection): boolean {
  return Boolean(latest && latest.revision === receipt.revision && latest.bindingId === receipt.bindingId);
}
export function memoryPolicySettled(baseRevision: number, desired: boolean, receipt?: MemoryBindingView, latest?: MemoryBindingView): boolean {
  return Boolean(receipt && latest && latest.revision === receipt.revision && receipt.policy.completedTurnSync === desired && (receipt.revision === baseRevision || receipt.revision === baseRevision + 1));
}

// Only the exact visible Agent/form version owns a foreground response.
export class MemoryRequestScope {
  private generation = 0;
  private disposed = false;
  invalidate() { this.generation++; }
  capture(): () => boolean { const generation = this.generation; return () => !this.disposed && generation === this.generation; }
  dispose() { this.disposed = true; this.invalidate(); }
}
