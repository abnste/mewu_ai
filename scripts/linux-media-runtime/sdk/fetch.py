# SPDX-License-Identifier: MPL-2.0
"""Fetch fixed official upstream archives; reject mismatches before unpacking."""
import concurrent.futures,hashlib,json,pathlib,ssl,time,urllib.request
root=pathlib.Path('/opt/mewu-gst-src')
manifest=json.loads((root/'sources.json').read_text(encoding='utf-8'))

def fetch(url,name,sha256=None,limit=256*1024*1024):
    assert url.startswith('https://') and pathlib.Path(name).name==name
    target=root/name;assert not target.exists();hashed=hashlib.sha256();total=0;started=time.monotonic()
    print('Fetching '+name,flush=True)
    with urllib.request.urlopen(url,context=ssl.create_default_context(),timeout=60) as response,target.open('xb') as output:
        assert response.url.startswith('https://')
        while block:=response.read(1024*1024):
            total+=len(block);assert total<=limit and time.monotonic()-started<=300;hashed.update(block);output.write(block)
    assert total>0
    if sha256:assert hashed.hexdigest()==sha256,name
    print('Verified '+name,flush=True)
def download(source):fetch(source['url'],source['file'],source['sha256'])
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
    list(pool.map(download,manifest['sources']))
fetch('https://ffmpeg.org/ffmpeg-devel.asc','ffmpeg-devel.asc',limit=1024*1024)
fetch('https://ffmpeg.org/releases/ffmpeg-7.1.5.tar.xz.asc','ffmpeg-7.1.5.tar.xz.asc',limit=1024*1024)