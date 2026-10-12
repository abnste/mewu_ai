# SPDX-License-Identifier: MPL-2.0
import ctypes
import hashlib
import json
import os
import difflib
import email.utils
from debian.changelog import Changelog
import subprocess
from pathlib import Path
from elftools.elf.elffile import ELFFile

src=Path('/packet/source/patched')
build=Path('/packet/build')
prefix=Path('/packet/replacement')
evidence=Path('/packet/evidence')
fax_source=src/'libtiff/tif_fax3.c'
before=fax_source.read_text(encoding='utf-8')
assert hashlib.sha256(fax_source.read_bytes()).hexdigest()=='39e9afb2666efbd47120a3c365ecf653ff2800bd2a4d3aa1c8536db45479dcff'
old_line='dsp->nruns = TIFFroundup_32(rowpixels,32);'
assert before.count(old_line)==1
after=before.replace(old_line,'dsp->nruns = TIFFroundup_32(rowpixels + 1,32);')
patch_text=''.join(difflib.unified_diff(before.splitlines(True),after.splitlines(True),fromfile='a/libtiff/tif_fax3.c',tofile='b/libtiff/tif_fax3.c'))
patch_path=evidence/'upstream-fax3-issue513-backport.patch'
patch_path.write_text(patch_text,encoding='utf-8')
subprocess.run(['patch','--batch','--fuzz=0','-p1','-i',str(patch_path)],cwd=src,check=True)
assert fax_source.read_text(encoding='utf-8')==after
backport={'upstreamCommit':'8a4f6b587be4fa7bb39fe17f5f9dec52182ab26e',
          'officialSource':'https://gitlab.com/libtiff/libtiff/-/commit/8a4f6b587be4fa7bb39fe17f5f9dec52182ab26e',
          'issue':'https://gitlab.com/libtiff/libtiff/-/issues/513',
          'reason':'Valid width32 alternating black/white fax3 fails in exact original and unmodified patched Ubuntu baseline. Preserve regression fixture; port only official core one-line fix, retaining all48 Ubuntu patches.',
          'beforeSha256':hashlib.sha256(before.encode()).hexdigest(),'afterSha256':hashlib.sha256(after.encode()).hexdigest(),
          'patchSha256':hashlib.sha256(patch_path.read_bytes()).hexdigest()}
(evidence/'upstream-backport.json').write_text(json.dumps(backport,indent=2)+'\n',encoding='utf-8')
# Upstream4.3 has no tiff-tools flag; compile only the shared library target.
# Symbol versioning/SONAME come from unmodified patched upstream configuration.
command=['cmake','-S',str(src),'-B',str(build),'-G','Ninja','-DCMAKE_BUILD_TYPE=Release','-DCMAKE_INSTALL_PREFIX='+str(prefix),'-DCMAKE_INSTALL_LIBDIR=lib','-DCMAKE_BUILD_WITH_INSTALL_RPATH=ON','-DCMAKE_INSTALL_RPATH=$ORIGIN','-DBUILD_SHARED_LIBS=ON','-Djbig=OFF','-Djpeg=ON','-Dold-jpeg=ON','-Dzlib=ON','-Dlibdeflate=ON','-Dlzma=ON','-Dzstd=ON','-Dwebp=ON','-Dlerc=OFF','-Dld-version-script=ON']
with (src/'debian/changelog').open(encoding='utf-8') as handle:
    changelog=Changelog(handle)
source_epoch=int(email.utils.parsedate_to_datetime(changelog[0].date).timestamp())
build_env=dict(os.environ,SOURCE_DATE_EPOCH=str(source_epoch))
subprocess.run(command,check=True,env=build_env)
subprocess.run(['cmake','--build',str(build),'--target','tiff','--parallel','2'],check=True,env=build_env)
prefix.joinpath('lib').mkdir(parents=True,exist_ok=True)
library=next(build.joinpath('libtiff').glob('libtiff.so.5.*'))
import shutil
shutil.copy2(library,prefix/'lib'/library.name)
(prefix/'lib/libtiff.so.5').symlink_to(library.name)
(prefix/'lib/libtiff.so').symlink_to(library.name)
def elf_record(path):
    with path.open('rb') as handle:
        elf=ELFFile(handle)
        dynamic=elf.get_section_by_name('.dynamic')
        tags=list(dynamic.iter_tags())
        symbols=elf.get_section_by_name('.dynsym')
        version=elf.get_section_by_name('.gnu.version_d')
        versions=[]
        if version:
            for definition, auxiliaries in version.iter_versions(): versions.extend(auxiliary.name for auxiliary in auxiliaries)
        return {'path':str(path),'bytes':path.stat().st_size,'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),
                'needed':[tag.needed for tag in tags if tag.entry.d_tag=='DT_NEEDED'],
                'soname':[tag.soname for tag in tags if tag.entry.d_tag=='DT_SONAME'],
                'runpath':[tag.runpath for tag in tags if tag.entry.d_tag=='DT_RUNPATH'],
                'definedSymbols':sorted({symbol.name for symbol in symbols.iter_symbols() if symbol.name and symbol['st_shndx']!='SHN_UNDEF'}),
                'symbolVersions':versions,
                'sectionHashes':{name:hashlib.sha256(elf.get_section_by_name(name).data()).hexdigest() for name in ('.text','.rodata') if elf.get_section_by_name(name)}}

new=elf_record(prefix/'lib'/library.name)
assert new['soname']==['libtiff.so.5'] and new['runpath']==['$ORIGIN']
assert set(new['needed'])=={'libz.so.1','libdeflate.so.0','libjpeg.so.8','liblzma.so.5','libzstd.so.1','libwebp.so.7','libm.so.6','libc.so.6'}
codec=ctypes.CDLL(str(prefix/'lib'/library.name))
codec.TIFFIsCODECConfigured.argtypes=[ctypes.c_uint16]
codec.TIFFIsCODECConfigured.restype=ctypes.c_int
schemes=[number for number in range(65536) if codec.TIFFIsCODECConfigured(number)]
expected=[1,2,3,4,5,6,7,8,32766,32771,32773,32809,32909,32946,34676,34677,34925,50000,50001]
assert schemes==expected,schemes
manifest={'sourcePackage':'tiff','sourceVersion':os.environ['TIFF_SOURCE_VERSION'],'libraryName':library.name,**new,
          'configuredCodecSchemes':schemes,'configuration':command,'sourceDateEpoch':source_epoch,'upstreamBackport':backport,
          'license':'libtiff BSD-style license; see exact COPYRIGHT',
          'capabilityChange':'JBIG TIFF codec intentionally unsupported; all19 other pinned codec factories retained.'}
(evidence/'library.json').write_text(json.dumps(manifest,indent=2)+'\n',encoding='utf-8')
print(json.dumps({key:manifest[key] for key in ('sourceVersion','libraryName','bytes','sha256','soname','needed','runpath','configuredCodecSchemes')}))
