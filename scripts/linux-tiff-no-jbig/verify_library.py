# SPDX-License-Identifier: MPL-2.0
import argparse
import json
import os
import subprocess
from pathlib import Path

parser=argparse.ArgumentParser()
parser.add_argument('--appdir',type=Path)
args=parser.parse_args()
source=Path('/packet/source/patched')
build=Path('/packet/build')
evidence=Path('/packet/evidence')
library_dir=args.appdir/'usr/lib' if args.appdir else Path('/packet/replacement/lib')
command=['cc','-O2','-Wall','-Wextra','-Werror','-I'+str(source/'libtiff'),'-I'+str(build/'libtiff'),'/packet/codec_fixture.c','-L/packet/replacement/lib','-Wl,-rpath,/packet/replacement/lib','-ltiff','-o','/packet/codec-fixture']
command.extend(subprocess.check_output(['pkg-config','--cflags','--libs','gtk+-3.0','gdk-pixbuf-2.0'],text=True).split())
subprocess.run(command,check=True)
env=dict(os.environ,LD_LIBRARY_PATH=str(library_dir))
query='/usr/lib/x86_64-linux-gnu/gdk-pixbuf-2.0/gdk-pixbuf-query-loaders'
loader=args.appdir/'usr/lib/gdk-pixbuf-2.0/2.10.0/loaders/libpixbufloader-tiff.so' if args.appdir else Path('/usr/lib/x86_64-linux-gnu/gdk-pixbuf-2.0/2.10.0/loaders/libpixbufloader-tiff.so')
cache=subprocess.check_output([query,str(loader)],env=env)
assert b'libpixbufloader-tiff.so' in cache
(evidence/'tiff-loaders.cache').write_bytes(cache)
env['GDK_PIXBUF_MODULE_FILE']=str(evidence/'tiff-loaders.cache')
result=subprocess.run(['xvfb-run','-a','/packet/codec-fixture'],env=env,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=30)
(evidence/'codec-fixture.log').write_text(result.stdout,encoding='utf-8')
print(result.stdout,flush=True)
result.check_returncode()
print(json.dumps({'codecFixtureExit':result.returncode,'appdirInput':str(args.appdir) if args.appdir else None,'scope':'11 TIFF codecs plus PNG/JPEG/GTK known-pixel fixture; no application GUI acceptance'}))
