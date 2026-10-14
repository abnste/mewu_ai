// SPDX-License-Identifier: MPL-2.0
import { definePlugin } from '@mewu/sdk';

function ruler() {
  // All interactive content is authored in this independently installable
  // renderer. The host supplies movement/refresh/close object affordances only.
  const ticks = Array.from({ length: 51 }, (_, i) => {
    const x = 20 + i * 8;
    const major = i % 5 === 0;
    return `<path d="M${x} 18v${major ? 24 : 12}"/>` +
      (major ? `<text x="${x}" y="60">${i / 5}</text>` : '');
  }).join('');
  return {
    width: 440, height: 120,
    html: `<!doctype html><html><head><meta charset="utf-8"><style>
      *{box-sizing:border-box}html,body{margin:0;background:transparent;overflow:hidden;color:#152029;font-family:system-ui}
      svg{display:block;width:440px;height:120px;touch-action:none}text{text-anchor:middle;font-size:11px;fill:currentColor;stroke:none}
      button{all:unset;cursor:pointer;position:absolute;right:19px;bottom:16px;width:28px;height:28px;text-align:center;border-radius:8px;font-size:19px}
      button:hover{background:#ffffff80}button:focus-visible{outline:2px solid #1875e4}
    </style></head><body><svg viewBox="0 0 440 120" xmlns="http://www.w3.org/2000/svg" aria-label="尺">
      <rect x="4" y="4" width="432" height="112" rx="18" fill="#f4cf79" fill-opacity=".9"/>
      <g stroke="currentColor" stroke-width="1.3">${ticks}</g>
      <path id="marker" d="M220 12v64" stroke="#c63442" stroke-width="2"/>
      <text id="value" x="220" y="99">5.00</text>
    </svg><button aria-label="归零" title="归零">↺</button><script>
      const svg=document.querySelector('svg'),marker=document.querySelector('#marker'),value=document.querySelector('#value');
      function move(x){x=Math.max(20,Math.min(420,x));marker.setAttribute('d','M'+x+' 12v64');value.textContent=((x-20)/40).toFixed(2)}
      svg.addEventListener('pointerdown',e=>{svg.setPointerCapture(e.pointerId);move(e.offsetX)});
      svg.addEventListener('pointermove',e=>{if(svg.hasPointerCapture(e.pointerId))move(e.offsetX)});
      svg.addEventListener('pointerup',e=>{if(svg.hasPointerCapture(e.pointerId))svg.releasePointerCapture(e.pointerId)});
      document.querySelector('button').addEventListener('click',()=>move(20));
    </script></body></html>`
  };
}

export default definePlugin(ctx => {
  ctx.renderers.register({ id: 'example.interactive-widget.ruler', title: '圆角尺', metadata: { transparent: true, interactive: true } }, ruler);
  ctx.ui.register({ id: 'example.interactive-widget.ui', title: '圆角尺', metadata: { transparent: true, interactive: true } }, ruler);
  ctx.commands.register({ id: 'example.interactive-widget.create', title: '圆角尺', metadata: { placement: 'space-toolbar' } },
    () => ctx.ui.render(ruler()));
});
