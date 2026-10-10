// SPDX-License-Identifier: MPL-2.0
import type { ConnectionParameters, ConnectionProfile } from './contracts';

export function normalizeConnectionPath(path?: string | null): string | null {
  const value = path?.trim(); return value ? value.startsWith('/') ? value : `/${value}` : null;
}

export function parseConnectionParameters(text: string): ConnectionParameters {
  if (text.length > 16384) throw new Error('请求参数过长');
  let value: unknown;
  try { value = JSON.parse(text.trim() || '{}'); } catch { throw new Error('请求参数 JSON 无效'); }
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('请求参数应为 JSON 对象');
  for (const [name, entry] of Object.entries(value)) {
    const valid = name === 'temperature' ? typeof entry === 'number' && Number.isFinite(entry) && entry >= 0 && entry <= 2
      : name === 'top_p' ? typeof entry === 'number' && Number.isFinite(entry) && entry > 0 && entry <= 1
        : name === 'service_tier' && (entry === 'standard' || entry === 'priority');
    if (!valid) throw new Error('仅支持 temperature（0–2）、top_p（0–1，不含 0）、service_tier（standard / priority）');
  }
  return value as ConnectionParameters;
}
export function validateConnectionProfile(profile: ConnectionProfile, requireModel = true) {
  if (!profile.name.trim() || [...profile.name.trim()].length > 80 || /[\x00-\x1f\x7f]/.test(profile.name)) throw new Error('连接名称需为 1 至 80 字，不能包含控制字符');
  if (!/^[a-z\d._-]{1,80}$/i.test(profile.providerId)) throw new Error('服务商标识无效');
  let url: URL;
  try { url = new URL(profile.baseUrl); } catch { throw new Error('请填写有效 API 地址'); }
  if (!['https:', 'http:'].includes(url.protocol) || url.username || url.password || url.search || url.hash || /[\s\x00-\x1f\x7f]/.test(profile.baseUrl) || profile.baseUrl.length > 4096) throw new Error('API 地址不能包含账号、查询参数、片段或空白');
  const loopback = url.hostname === 'localhost' || url.hostname === '[::1]' || /^127(?:\.\d{1,3}){3}$/.test(url.hostname);
  if (url.protocol === 'http:' && !loopback) throw new Error('远程连接需使用 HTTPS');
  if (requireModel && !profile.model.trim()) throw new Error('请填写模型');
  if ([...profile.model].length > 256 || /[\s\x00-\x1f\x7f]/.test(profile.model)) throw new Error('模型名称过长或包含空白');
  if (!['chat_completions', 'anthropic_messages', 'openai_responses'].includes(profile.advanced.protocol)) throw new Error('API 格式无效');
  if (!['bearer', 'api_key', 'none'].includes(profile.advanced.authMode)) throw new Error('认证方式无效');
  const path = profile.advanced.requestPath?.trim();
  if (path && (path.length > 512 || !/^[a-z\d/_\-.]+$/i.test(path) || path.startsWith('//') || path.split('/').some(part => part === '.' || part === '..'))) throw new Error('请求路径需为域名根目录下的 API 路径');
  parseConnectionParameters(JSON.stringify(profile.advanced.requestParameters));
  if (profile.advanced.protocol === 'anthropic_messages') {
    if ((profile.advanced.requestParameters.temperature ?? 0) > 1) throw new Error('Anthropic 的 temperature 最大为 1');
    if (profile.advanced.requestParameters.service_tier === 'priority') throw new Error('Anthropic 暂不支持 priority');
  }
}
