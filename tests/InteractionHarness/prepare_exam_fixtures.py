# SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
# SPDX-License-Identifier: MPL-2.0
"""Opt-in fixtures on a separately downloaded official paper; no exam redistribution.

Render TSA2025_9MC2_Q.pdf page 11 with Poppler -scale-to 1600 -singlefile
to .codex-build/teaching-evaluation/paper-2-page11.png before running.
"""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont
import json

folder = Path('.codex-build/teaching-evaluation')
source = Image.open(folder / 'paper-2-page11.png').convert('RGB')
if source.size != (1132, 1600):
    raise ValueError('Expected the official page rendered at 1132 x 1600 pixels')
font = ImageFont.truetype('C:/Windows/Fonts/arial.ttf', 30)
label = ImageFont.truetype('C:/Windows/Fonts/msyh.ttc', 23)
cases = {
    'student-A': ['x = 2', '-5', '5.6 x 10^-4', '5b', '(2x - y)(3a - 5)', '4x^2 + 1'],
    'student-B': ['x = 14/3', '-0.2', '5.6 x 10^4', '5b', '', '4x^2 + 1'],
}
manifest = []
for name, answers in cases.items():
    paper = source.crop((60, 660, 1070, 1320))
    image = Image.new('RGB', (1010, 740), 'white')
    image.paste(paper, (0, 65))
    draw = ImageDraw.Draw(image)
    draw.text((20, 12), f'{name} | 2025 TSA 9MC2 第11页节选 | 蓝字为模拟作答',
              font=label, fill='#555555')
    items = []
    for index, answer in enumerate(answers):
        y = 136 + index * 103
        if answer:
            draw.text((495, y), answer, font=font, fill='#173E95')
        items.append({'question': 24 + index, 'studentAnswer': answer,
                      'answerBox': [495/1010, y/740, 450/1010, 39/740]})
    image.save(folder / (name + '.png'))
    manifest.append({'id': name, 'file': name + '.png', 'items': items})
(folder / 'fixtures.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding='utf-8')
print('Prepared two explicitly simulated student responses on an official exam excerpt.')
