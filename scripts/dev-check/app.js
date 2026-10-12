// SPDX-License-Identifier: MPL-2.0
const labels = {pending: '待执行', running: '执行中', passed: '通过', failed: '失败', blocked: '阻断', interrupted: '中断'};
const actionLabels = {typecheck: '类型检查', 'frontend-build': '前端构建', tests: '相关测试'};
const kindLabels = {'browser-gui': '浏览器交互', 'user-windows': '用户 Windows 验收', 'windows-update': '本机更新', 'docker-gui': 'Docker 桌面交互', 'mac-gui': 'Mac 实机交互', build: '仅构建'};
let snapshot; let initialized = false; let polling = false;
const el = (tag, text, className) => { const node = document.createElement(tag); if (text !== undefined) node.textContent = text; if (className) node.className = className; return node; };
function lane(name, label) {
  const data = snapshot.lanes[name]; const card = el('article', undefined, 'lane');
  card.append(el('h3', label), el('span', labels[data.status] || data.status, `status ${data.status}`));
  if (data.source) {
    card.append(el('p', `${kindLabels[data.kind] || data.kind} · ${data.source.slice(0, 10)}`, 'muted'));
    if (data.source !== snapshot.currentSource.head) card.append(el('p', '源版本已变化', 'warning'));
    else if (data.fingerprint && data.fingerprint !== snapshot.currentSource.fingerprint) card.append(el('p', '源文件指纹已变化', 'warning'));
    else if (!data.fingerprint && snapshot.currentSource.dirty) card.append(el('p', '当前还有未提交改动，回执只代表指定提交', 'warning'));
    card.append(el('p', `外部回执 · SHA256 ${data.evidenceSha256.slice(0, 12)}…`, 'muted'));
    if (data.browserUrl) { const link = el('a', '打开本轮交互页面'); link.href = data.browserUrl; link.target = '_blank'; link.rel = 'noopener'; card.append(link); }
  }
  if (name === 'mac') { const health = snapshot.macHealth; card.append(el('p', `SSH ${health.ssh} · 桌面 ${health.desktop}${health.stale ? ' · 状态已过期' : ''}`, 'muted')); }
  return card;
}
function render() {
  const src = snapshot.currentSource;
  document.querySelector('#source').textContent = `${src.branch} · ${src.head.slice(0, 10)}${src.dirty ? ' · 有未提交修改' : ''} · 文件指纹 ${src.fingerprint.slice(0, 12)} · 基线 ${new Date(src.checkedAt).toLocaleTimeString()}`;
  document.querySelector('#connection').textContent = snapshot.busy ? '检查执行中' : '已连接';
  document.querySelectorAll('.actions button').forEach(button => { button.disabled = snapshot.busy; });
  if (!initialized) {
    const container = document.querySelector('#tests');
    for (const name of snapshot.tests) {
      const label = el('label'); const input = el('input'); input.type = 'checkbox'; input.value = name;
      input.addEventListener('change', () => { document.querySelector('#selected-count').textContent = document.querySelectorAll('#tests input:checked').length; });
      label.append(input, document.createTextNode(name.replace('.test.mjs', ''))); container.append(label);
    }
    initialized = true;
  }
  document.querySelector('#daily-lanes').replaceChildren(lane('browser', '浏览器检查'), lane('windows', 'Windows 更新 / 用户验收'));
  document.querySelector('#release-lanes').replaceChildren(lane('linux', 'Linux Docker'), lane('mac', '内网 Mac'));
  const jobs = snapshot.jobs.slice(-20).reverse().map(job => {
    const row = el('article', undefined, 'job');
    row.append(el('strong', actionLabels[job.action] || job.action), el('span', labels[job.status] || job.status, `status ${job.status}`));
    row.append(el('p', `${job.source.head.slice(0, 10)} · ${job.source.fingerprint.slice(0, 12)} · ${new Date(job.startedAt).toLocaleTimeString()}${job.elapsedMs !== undefined ? ` · ${(job.elapsedMs / 1000).toFixed(1)} 秒 · 退出码 ${job.exitCode}` : ''}`, 'muted'));
    if (job.tests.length) row.append(el('p', job.tests.map(test => test.replace('.test.mjs', '')).join('、'), 'muted'));
    if (job.stale || job.source.fingerprint !== src.fingerprint) row.append(el('p', '源文件已变化，此结果不能代表当前版本', 'warning'));
    if (job.error) row.append(el('p', job.error, 'warning'));
    if (job.logSha256) { const link = el('a', '日志'); link.href = `/logs/${job.id}`; link.target = '_blank'; link.rel = 'noopener'; row.append(link); }
    return row;
  });
  document.querySelector('#jobs').replaceChildren(...jobs);
}
async function refresh() {
  if (polling) return;
  polling = true;
  try {
    const response = await fetch('/api/state'); if (!response.ok) throw Error('无法读取检查状态');
    snapshot = await response.json(); render();
  } catch (error) { document.querySelector('#connection').textContent = '连接中断'; document.querySelector('#error').textContent = error.message; }
  finally { polling = false; }
}
async function run(action, tests) {
  document.querySelector('#error').textContent = '';
  try {
    const response = await fetch('/api/run', {method: 'POST', headers: {'Content-Type': 'application/json', 'X-Mewu-CSRF': snapshot.csrf}, body: JSON.stringify({action, ...(tests ? {tests} : {})})});
    const result = await response.json(); if (!response.ok) throw Error(result.error || '检查启动失败');
    await refresh();
  } catch (error) { document.querySelector('#error').textContent = error.message; }
}
document.querySelectorAll('[data-action]').forEach(button => button.addEventListener('click', () => run(button.dataset.action)));
document.querySelector('#run-tests').addEventListener('click', () => run('tests', [...document.querySelectorAll('#tests input:checked')].map(input => input.value)));
refresh(); setInterval(refresh, 4000);
