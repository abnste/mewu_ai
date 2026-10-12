# SPDX-License-Identifier: MPL-2.0
"""Export locked media runtime plus a separately stored host ABI closure.

The portable launcher selects native media libraries, not an older GTK/GLib or
glibc over a new host desktop. Final AppDir integration additionally validates
its matching distro dependency closure. Does not run the product application.
"""
import argparse, hashlib, json, os, pathlib, shutil, stat
from elftools.elf.elffile import ELFFile

def digest(path):
    before=path.stat()
    with path.open('rb') as handle:
        opened=os.fstat(handle.fileno());result=hashlib.file_digest(handle,'sha256').hexdigest() if hasattr(hashlib,'file_digest') else hashlib.sha256(handle.read()).hexdigest()
    after=path.stat()
    assert (before.st_dev,before.st_ino,before.st_size,before.st_mtime_ns)==(opened.st_dev,opened.st_ino,opened.st_size,opened.st_mtime_ns)==(after.st_dev,after.st_ino,after.st_size,after.st_mtime_ns)
    return result
def soname(path):
    with path.open('rb') as handle:
        elf=ELFFile(handle);section=elf.get_section_by_name('.dynamic')
        names=[tag.soname for tag in section.iter_tags() if tag.entry.d_tag=='DT_SONAME'] if section else []
    assert len(names)<=1
    if names:assert pathlib.Path(names[0]).name==names[0]
    return names[0] if names else None
def copy_file(source,destination,mode=0o644):
    source=source.resolve(strict=True);assert source.is_file() and source.stat().st_size<=512*1024*1024
    destination.parent.mkdir(parents=True,exist_ok=True)
    assert not destination.exists() and not destination.is_symlink(),str(destination)
    shutil.copyfile(source,destination);destination.chmod(mode)
    assert digest(source)==digest(destination)
def copy_tree(source,destination):
    assert source.is_dir() and not destination.exists()
    for path in sorted(source.rglob('*')):
        if path.is_dir():continue
        resolved=path.resolve(strict=True);assert resolved.is_relative_to(source.resolve()),str(path)
        copy_file(resolved,destination/path.relative_to(source),0o644)
def library(source,directory):
    source=source.resolve(strict=True);target=directory/source.name
    if not target.exists():copy_file(source,target)
    else:assert digest(source)==digest(target)
    alias=soname(source)
    if alias and alias!=target.name:
        link=directory/alias
        if link.is_symlink():assert link.resolve()==target.resolve()
        else:assert not link.exists();link.symlink_to(target.name)
def export(prefix,destination,launcher):
    prefix=prefix.resolve(strict=True);assert prefix.is_dir()
    destination=destination.absolute();assert not destination.exists() and not destination.is_symlink()
    assert not destination.is_relative_to(prefix)
    closure=json.loads((prefix/'share/mewu-sdk-audit/closure.json').read_text(encoding='utf-8'))
    assert '1.28.8' in closure['sdkRuntimeVersion'] and closure['ffmpegLicense']=='LGPL version 2.1 or later'
    assert tuple(map(int,closure['maxRequiredGlibcVersion'].split('.')))<=(2,35)
    destination.mkdir(mode=0o755)
    lib=destination/'usr/lib';audit=destination/'audit/sysroot/lib';lib.mkdir(parents=True);audit.mkdir(parents=True)
    copied_libraries=[]
    for entry in closure['dynamicLibraries']:
        source=pathlib.Path(entry['file']).resolve(strict=True);assert digest(source)==entry['sha256']
        # Retain every library as audit evidence; only native media and its
        # mandatory AAC library enter the launcher's search directory.
        library(source,audit)
        selected=source.is_relative_to(prefix) or source.name.startswith('libvo-aacenc.so.')
        if selected:library(source,lib)
        copied_libraries.append({'source':str(source),'sha256':entry['sha256'],'soname':soname(source),'selectedForPortableLaunch':selected})
    for entry in closure['runtimeRoots']:
        source=pathlib.Path(entry['file']).resolve(strict=True);assert source.is_relative_to(prefix)
        relative=source.relative_to(prefix)
        if relative.as_posix().startswith('libexec/gstreamer-1.0/'):
            target=destination/'usr/lib/gstreamer1.0/gstreamer-1.0'/source.name
            copy_file(source,target,0o755)
        elif relative.parent==pathlib.Path('lib') and soname(source):library(source,lib)
        else:
            target=destination/'usr'/relative
            if target.exists():assert target.is_file() and not target.is_symlink() and digest(source)==digest(target)
            else:copy_file(source,target)
    helper=prefix/'libexec/gstreamer-1.0/gst-ptp-helper'
    assert helper.is_file() and helper.stat().st_mode&0o111
    assert helper.stat().st_mode&0o6000==0,'PTP helper must have no setuid/setgid'
    helper_target=destination/'usr/lib/gstreamer1.0/gstreamer-1.0/gst-ptp-helper'
    if helper_target.exists():assert not helper_target.is_symlink() and digest(helper)==digest(helper_target) and helper_target.stat().st_mode&0o111
    else:copy_file(helper,helper_target,0o755)
    for name in ('gst-inspect-1.0','gst-launch-1.0'):
        copy_file(prefix/'bin'/name,destination/'usr/bin'/name,0o755)
    copy_tree(prefix/'share/pipewire',destination/'usr/share/pipewire')
    notices=destination/'usr/share/doc/mewu-media-runtime'
    copy_tree(prefix/'share/mewu-sdk-audit',notices)
    copy_file(launcher,destination/'run-with-media',0o755)
    # Product packaging reads this separate manifest, never a private SDK path.
    manifest={'schemaVersion':1,'kind':'mewu-linux-media-runtime','gstreamerVersion':'1.28.8','ffmpegVersion':'7.1.5','maxRequiredGlibc':'2.35','sourceManifest':closure['sources'],'libraries':copied_libraries,'portablePolicy':'Only native media plus VO AAC enter LD_LIBRARY_PATH; distro GTK/GLib/libc/C++ stay on the host. Full audited library closure is stored separately for matched AppDir integration.','files':[]}
    for path in sorted(destination.rglob('*')):
        if path.is_dir():continue
        relative=path.relative_to(destination).as_posix()
        if path.is_symlink():manifest['files'].append({'path':relative,'kind':'symlink','target':os.readlink(path)});continue
        manifest['files'].append({'path':relative,'kind':'file','bytes':path.stat().st_size,'sha256':digest(path),'mode':stat.S_IMODE(path.stat().st_mode)})
    (notices/'runtime.json').write_text(json.dumps(manifest,indent=2)+'\n',encoding='utf-8')
    print(json.dumps({'destination':str(destination),'gstreamerVersion':manifest['gstreamerVersion'],'files':len(manifest['files']),'closedLibraries':len(copied_libraries),'portableLibraries':sum(entry['selectedForPortableLaunch'] for entry in copied_libraries),'applicationExecuted':False}))
if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--prefix',type=pathlib.Path,required=True);parser.add_argument('--destination',type=pathlib.Path,required=True);parser.add_argument('--launcher',type=pathlib.Path,required=True)
    args=parser.parse_args();export(args.prefix,args.destination,args.launcher)
