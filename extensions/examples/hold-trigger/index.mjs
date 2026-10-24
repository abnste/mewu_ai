// SPDX-License-Identifier: MPL-2.0
import { definePlugin } from '@mewu/sdk';

const phaseLabel = { down: '按下', up: '松开', cancel: '取消' };
const reasonLabel = { escape: 'Esc', blur: '窗口失焦' };
const phaseColor = { down: '#1875e4', up: '#2f8f4e', cancel: '#c63442' };

// One small badge per event keeps every arrival visible on the canvas; the
// space's normal object affordances move or delete them.
function badge(event) {
  const phase = phaseLabel[event.phase] ?? event.phase;
  const reason = event.reason ? `（${reasonLabel[event.reason] ?? event.reason}）` : '';
  const held = [
    event.modifiers.ctrl ? 'Ctrl' : '',
    event.modifiers.shift ? 'Shift' : '',
    event.modifiers.alt ? 'Alt' : '',
  ].filter(Boolean).join('+') || '无修饰键';
  const color = phaseColor[event.phase] ?? '#152029';
  return {
    width: 264, height: 88,
    html: `<!doctype html><html><head><meta charset="utf-8"><style>
      *{box-sizing:border-box}html,body{margin:0;background:transparent;overflow:hidden;color:#152029;font-family:system-ui}
      div{padding:10px 14px;border-radius:12px;background:#ffffffea;box-shadow:0 4px 14px #0003;font-size:13px;line-height:1.6}
      strong{font-size:15px;color:${color}}small{color:#4a5a66}
    </style></head><body><div aria-label="触发器事件">
      <strong>Alt 触发器 · ${phase}${reason}</strong><br>
      <small>指针 x=${Math.round(event.pointer.x)} y=${Math.round(event.pointer.y)} 按钮=${event.pointer.buttons} · ${held}</small>
    </div></body></html>`
  };
}

export default definePlugin(ctx => {
  ctx.triggers.register({ id: 'example.hold-trigger.alt', code: 'Alt' },
    event => ctx.ui.render(badge(event)));
});
