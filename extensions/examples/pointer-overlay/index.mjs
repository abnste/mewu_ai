// SPDX-License-Identifier: MPL-2.0
import { definePlugin } from '@mewu/sdk';

function crosshair() {
  // A self-contained translucent crosshair: no external assets, only SVG.
  const ticks = Array.from({ length: 13 }, (_, i) => {
    const major = i % 4 === 0;
    return `<path d="M${8 + i * 20} 224v${major ? 10 : 5}"/>` +
      (major ? `<text x="${8 + i * 20}" y="248">${(i - 6) * 40}</text>` : '');
  }).join('');
  return {
    width: 260, height: 260,
    html: `<!doctype html><html><head><meta charset="utf-8"><style>
      *{box-sizing:border-box}html,body{margin:0;background:transparent;overflow:hidden;color:#152029;font-family:system-ui}
      svg{display:block;width:260px;height:260px}
      text{text-anchor:middle;font-size:10px;fill:currentColor;stroke:none}
    </style></head><body><svg viewBox="0 0 260 260" xmlns="http://www.w3.org/2000/svg" aria-label="十字标">
      <circle cx="130" cy="130" r="96" fill="#f4cf79" fill-opacity=".22"/>
      <circle cx="130" cy="130" r="48" fill="none" stroke="#b47d1b" stroke-opacity=".45" stroke-dasharray="4 5"/>
      <path d="M130 6v72M130 182v72M6 130h72M182 130h72" stroke="#c63442" stroke-width="1.6" stroke-opacity=".85"/>
      <g stroke="#b47d1b" stroke-opacity=".55" stroke-width="1">${ticks}</g>
      <rect x="58" y="112" width="144" height="26" rx="13" fill="#ffffff" fill-opacity=".88"/>
      <text id="value" x="130" y="129" font-size="13">移动指针</text>
    </svg><script>
      const value=document.querySelector('#value');
      // Samples arrive while the frame itself stays transparent to the pointer.
      mewu.on('pointer',sample=>{value.textContent=Math.round(sample.x)+'，'+Math.round(sample.y);});
    </script></body></html>`
  };
}

export default definePlugin(ctx => {
  ctx.commands.register({ id: 'example.pointer-overlay.crosshair', title: '十字标', metadata: { placement: 'space-toolbar' } },
    async () => {
      // The overlay outlives this invocation: the host owns follow-through
      // and destroys it on pointerup, Escape or window blur.
      await ctx.ui.transient(crosshair());
      return { content: '十字标已弹出，移动指针查看坐标' };
    });
});
