# SPDX-License-Identifier: MPL-2.0
"""Stage an audited media runtime into a newly generated Tauri AppDir.

Build-only. Never executes the product or modifies the original AppDir. A
matched distro GTK/WebKit bundle is required; libc and graphics remain host
providers. Every replacement and loader path is recorded for release review.
"""
import argparse, hashlib, json, os, pathlib, shutil, stat, subprocess
from elftools.elf.elffile import ELFFile

FORBIDDEN = {'libjbig.so.0','libgpm.so.2','libslang.so.2','libaa.so.1','libcaca.so.0',
             'libfaad.so.2','libx264.so.163','libx265.so.199','libpostproc.so.55'}
# These providers must follow the installed graphics driver/compositor. SDK
# closure discovery must not undo linuxdeploy's host-driver exclusions.
# https://github.com/AppImageCommunity/pkg2appimage/blob/master/excludelist
HOST_GRAPHICS = {'libGL.so.1','libEGL.so.1','libGLX.so.0','libGLdispatch.so.0',
                 'libGLESv2.so.2','libGLESv1_CM.so.1',
                 'libOpenGL.so.0','libglapi.so.0','libgbm.so.1','libdrm.so.2',
                 'libEGL_mesa.so.0','libGLX_mesa.so.0',
                 'libX11.so.6','libX11-xcb.so.1','libxcb.so.1',
                 'libwayland-client.so.0','libwayland-cursor.so.0',
                 'libwayland-egl.so.1','libwayland-server.so.0'}
# System OCR's curl and the bundled Soup client share this stable SONAME.
# A copied older nghttp2 shadows newer host-curl API requirements.
# Keep the provider with the host, as with existing driver/runtime boundaries.
HOST_NETWORK = {'libnghttp2.so.14'}
HOST_LIBS = {'libc.so.6','libm.so.6','libpthread.so.0','libdl.so.2','librt.so.1',
             'ld-linux-x86-64.so.2','libgcc_s.so.1','libstdc++.so.6'} | HOST_GRAPHICS | HOST_NETWORK

def digest(path):
    with path.open('rb') as handle:
        value=hashlib.sha256()
        for block in iter(lambda:handle.read(1024*1024),b''): value.update(block)
    return value.hexdigest()

def record(path):
    with path.open('rb') as handle:
        if handle.read(4)!=b'\x7fELF': return None
        handle.seek(0); elf=ELFFile(handle); dynamic=elf.get_section_by_name('.dynamic')
        tags=list(dynamic.iter_tags()) if dynamic else []
        return {'sha256':digest(path), 'needed':[x.needed for x in tags if x.entry.d_tag=='DT_NEEDED'],
                'soname':[x.soname for x in tags if x.entry.d_tag=='DT_SONAME'],
                'runpath':[x.runpath for x in tags if x.entry.d_tag=='DT_RUNPATH']}

def scan(root):
    return {p.relative_to(root).as_posix():entry for p in root.rglob('*')
            if p.is_file() and not p.is_symlink() and (entry:=record(p))}

def safe_tree(root):
    count=0
    for path in [root,*root.rglob('*')]:
        count+=1; assert count<=4096, 'AppDir file budget exceeded'
        assert len(path.relative_to(root).parts)<=16
        mode=path.lstat().st_mode
        assert path.resolve(strict=True).is_relative_to(root.resolve()),str(path)
        if path.is_symlink():
            assert not pathlib.Path(os.readlink(path)).is_absolute(),str(path)
        else:
            assert stat.S_ISDIR(mode) or stat.S_ISREG(mode),str(path)
            path.chmod(stat.S_IMODE(mode)&~0o6022)

def desktop_roots(destination):
    # libappindicator-sys uses dlopen; it is absent from the main DT_NEEDED
    # graph. Copy the matched build-distro tray family, never host24's new
    # Ayatana into the older bundled GLib/GTK closure.
    providers={name:path for path,e in scan(destination).items() for name in e['soname']}
    search=[pathlib.Path('/usr/lib/x86_64-linux-gnu'),pathlib.Path('/lib/x86_64-linux-gnu')]
    copied=[];pending=['libayatana-appindicator3.so.1'];visited=set()
    while pending:
        name=pending.pop()
        assert pathlib.Path(name).name==name
        if name in visited:continue
        visited.add(name)
        if name in providers or name in HOST_LIBS:continue
        choices=[directory/name for directory in search if (directory/name).is_file()]
        assert choices,'Missing matched desktop runtime: '+name
        source=choices[0].resolve(strict=True)
        metadata=source.stat();assert metadata.st_uid==0 and not metadata.st_mode&0o022
        entry=record(source);assert entry and entry['soname']==[name],name
        assert not(set(entry['needed'])&FORBIDDEN),name
        target=destination/'usr/lib'/source.name;assert not target.exists()
        shutil.copyfile(source,target);target.chmod(0o644)
        subprocess.run(['patchelf','--set-rpath','$ORIGIN',str(target)],check=True)
        alias=target.parent/name
        if alias!=target:assert not alias.exists() and not alias.is_symlink();alias.symlink_to(target.name)
        owners=[]
        for candidate in (source,pathlib.Path(str(source)[4:]) if str(source).startswith('/usr/lib/') else source):
            if not candidate.is_file() or candidate.resolve()!=source:continue
            owner=subprocess.run(['dpkg-query','-S',str(candidate)],text=True,capture_output=True)
            if owner.returncode==0:owners.append(owner.stdout.strip());break
        assert owners,'Missing exact package ownership: '+str(source)
        package=owners[0].rsplit(': ',1)[0];package_name=package.split(':',1)[0]
        copyright=pathlib.Path('/usr/share/doc')/package_name/'copyright';assert copyright.is_file()
        notice=destination/'usr/share/doc'/package_name/'copyright';notice.parent.mkdir(parents=True,exist_ok=True)
        if notice.exists():assert notice.read_bytes()==copyright.read_bytes(),package_name
        else:shutil.copyfile(copyright,notice);notice.chmod(0o644)
        copied.append({'path':target.relative_to(destination).as_posix(),'sourceSha256':entry['sha256'],
                       'package':package,'packageVersion':subprocess.check_output(['dpkg-query','-W','-f=${Version}',package],text=True),
                       'copyrightSha256':digest(copyright),**record(target)})
        providers[name]=target.relative_to(destination).as_posix();pending.extend(entry['needed'])
    return copied
def stage(appdir,runtime,destination,receipt):
    appdir=appdir.resolve(strict=True);runtime=runtime.resolve(strict=True)
    destination=destination.absolute();assert not destination.exists() and not destination.is_symlink()
    assert not destination.is_relative_to(appdir) and not destination.is_relative_to(runtime)
    manifest=json.loads((runtime/'usr/share/doc/mewu-media-runtime/runtime.json').read_text())
    assert manifest['kind']=='mewu-linux-media-runtime' and manifest['gstreamerVersion']=='1.28.8'
    for entry in manifest['files']:
        path=runtime/entry['path'];assert path.resolve(strict=True).is_relative_to(runtime)
        if entry['kind']=='file':assert digest(path)==entry['sha256'],str(path)
        else:assert path.is_symlink() and os.readlink(path)==entry['target'],str(path)
    before=scan(appdir);assert 'usr/bin/MewuAI' in before
    shutil.copytree(appdir,destination,symlinks=True)
    removed=[]
    # Entire old media plugin/helper trees must be replaced, not mixed with a
    # distribution plugin hook that silently deploys the old core and codecs.
    for relative in ('usr/lib/gstreamer-1.0','usr/lib/gstreamer1.0','usr/lib/pipewire-0.3','usr/lib/spa-0.2','usr/share/pipewire'):
        target=destination/relative
        if target.exists():assert target.is_dir() and not target.is_symlink();shutil.rmtree(target)
    replaced=[]
    new_roots={}
    for source in sorted((runtime/'usr').rglob('*')):
        relative=source.relative_to(runtime);target=destination/relative
        if source.is_dir():target.mkdir(parents=True,exist_ok=True);continue
        if target.exists() or target.is_symlink():
            assert target.is_file() or target.is_symlink();target.unlink()
        target.parent.mkdir(parents=True,exist_ok=True)
        if source.is_symlink():target.symlink_to(os.readlink(source));continue
        shutil.copyfile(source,target);target.chmod(stat.S_IMODE(source.stat().st_mode))
        entry=record(target)
        if entry:
            loader=os.path.relpath(destination/'usr/lib',target.parent).replace(os.sep,'/')
            runpath='$ORIGIN' if loader=='.' else '$ORIGIN/'+loader
            subprocess.run(['patchelf','--set-rpath',runpath,str(target)],check=True)
            replaced.append({'path':relative.as_posix(),'sourceSha256':entry['sha256'],**record(target)})
            new_roots[relative.as_posix()]=record(target)
    # Old FFmpeg ABI/plugin-only GPL providers are removed only after the new
    # roots have been staged and an exact DT_NEEDED check can reject survivors.
    entries=scan(destination)
    obsolete={p for p,e in entries.items() if set(e['soname'])&(FORBIDDEN|HOST_GRAPHICS|HOST_NETWORK)
        or any(name.startswith(('libavcodec.so.58','libavformat.so.58','libavutil.so.56','libavfilter.so.7','libswresample.so.3','libswscale.so.5')) for name in e['soname'])}
    assert not any(set(e['needed'])&FORBIDDEN for p,e in entries.items() if p not in obsolete), 'Forbidden codec still reachable'
    targets={(destination/p).resolve() for p in obsolete}
    for target in destination.rglob('*'):
        if target.is_symlink() and target.resolve() in targets:target.unlink()
    for relative in sorted(obsolete):(destination/relative).unlink();removed.append(relative)
    # Keep the existing distro provider where linuxdeploy has applied a local
    # RUNPATH. Missing non-host runtime closure providers come from the exact
    # audited SDK, never an unreviewed guest path.
    providers={name:p for p,e in scan(destination).items() for name in e['soname']}
    for entry in manifest['libraries']:
        name=entry['soname']
        if name in HOST_LIBS or name in providers:continue
        source=runtime/'audit/sysroot/lib'/pathlib.Path(entry['source']).name
        assert source.is_file() and digest(source)==entry['sha256']
        target=destination/'usr/lib'/source.name;shutil.copyfile(source,target);target.chmod(0o644)
        subprocess.run(['patchelf','--set-rpath','$ORIGIN',str(target)],check=True)
        if name!=source.name:
            link=target.parent/name;assert not link.exists() and not link.is_symlink();link.symlink_to(source.name)
        providers[name]=target.relative_to(destination).as_posix()
    desktop_added=desktop_roots(destination)
    # Trace real roots after plugin replacement, then remove library-only
    # leftovers deployed for obsolete plugins (e.g. libchromaprint -> avcodec58).
    entries=scan(destination)
    by_name={name:path for path,e in entries.items() for name in e['soname']}
    for path,e in new_roots.items():
        for name in e['soname']:by_name[name]=path
    roots={path for path,e in entries.items() if pathlib.PurePosixPath(path).parent.as_posix()!='usr/lib' or 'libayatana-appindicator3.so.1' in e['soname']}
    retained=set();pending=list(roots)
    while pending:
        path=pending.pop()
        if path in retained:continue
        retained.add(path)
        for name in entries[path]['needed']:
            if name in by_name:pending.append(by_name[name])
    unused={path for path,e in entries.items() if e['soname'] and path not in retained}
    targets={(destination/path).resolve() for path in unused}
    for path in destination.rglob('*'):
        if path.is_symlink() and path.resolve() in targets:path.unlink()
    for relative in sorted(unused):(destination/relative).unlink();removed.append(relative)
    assert not any(set(e['soname']) & HOST_NETWORK for e in scan(destination).values()), 'Host network provider must not be bundled'
    providers={name:path for path,e in scan(destination).items() for name in e['soname']}
    # linuxdeploy may create several physical filenames with the same SONAME.
    # Pruning keeps one provider; restore its canonical DT_NEEDED filename so
    # a newer host Pango/GTK never falls through into the old bundled GLib.
    for name,relative in providers.items():
        if pathlib.PurePosixPath(relative).parent.as_posix()!='usr/lib':continue
        assert pathlib.Path(name).name==name
        target=destination/relative;alias=destination/'usr/lib'/name
        if alias.exists() or alias.is_symlink():
            assert alias.resolve(strict=True)==target.resolve(strict=True),name
        else:alias.symlink_to(target.name)
    # All surviving Gst family providers (including WebKit's GL/pbutils/FFT
    # imports) must originate in this exact audited runtime. SONAME alone is
    # insufficient: the old distro provider has the same SONAME.
    gst_providers={name:path for name,path in providers.items() if name.startswith('libgst')}
    replacements_by_path={entry['path']:entry for entry in replaced}
    for name,relative in gst_providers.items():
        source_path=runtime/'usr/lib'/name if pathlib.PurePosixPath(relative).parent.as_posix()=='usr/lib' else runtime/relative
        source=source_path.resolve(strict=True)
        target=(destination/relative).resolve(strict=True)
        proof=replacements_by_path[target.relative_to(destination).as_posix()]
        assert proof['sourceSha256']==digest(source),name
        assert proof['sha256']==digest(target),name
    # Existing launch hooks remain intact; insert owned relative PipeWire
    # settings through the normal AppRun hook mechanism.
    hook=destination/'apprun-hooks/mewu-media.sh';hook.parent.mkdir(exist_ok=True)
    hook.write_text('''#!/bin/sh
# SPDX-License-Identifier: MPL-2.0
export PIPEWIRE_MODULE_DIR="$APPDIR/usr/lib/pipewire-0.3"
export SPA_PLUGIN_DIR="$APPDIR/usr/lib/spa-0.2"
export PIPEWIRE_CONFIG_DIR="$APPDIR/usr/share/pipewire"
export PIPEWIRE_CONFIG_NAME=client.conf
''',encoding='utf-8');hook.chmod(0o755)
    apprun=destination/'AppRun';launcher=apprun.read_text(encoding='utf-8')
    root_marker='this_dir="$(readlink -f "$(dirname "$0")")"'
    assert launcher.count(root_marker)==1,'Unknown linuxdeploy AppRun root contract'
    # AppImage runtime sets APPDIR, but a legitimate extracted AppRun must be
    # coherent before the Gst hook (GTK's workaround runs after that hook).
    launcher=launcher.replace(root_marker,root_marker+'\nexport APPDIR="$this_dir"')
    invoke='exec "$this_dir"/AppRun.wrapped "$@"'
    assert launcher.count(invoke)==1,'Unknown linuxdeploy AppRun hook contract'
    apprun.write_text(launcher.replace(invoke,'source "$this_dir"/apprun-hooks/"mewu-media.sh"\n'+invoke),encoding='utf-8')
    safe_tree(destination)
    after=scan(destination)
    assert after['usr/bin/MewuAI']['sha256']==before['usr/bin/MewuAI']['sha256']
    assert all(not(set(e['needed'])&FORBIDDEN) for e in after.values())
    assert not any(set(e['soname'])&HOST_GRAPHICS for e in after.values())
    # Prove the trusted worker does not depend on the inherited LD_* env.
    assert '$ORIGIN/../lib' in ':'.join(after['usr/bin/MewuAI']['runpath']),after['usr/bin/MewuAI']['runpath']
    evidence={'schemaVersion':1,'scope':'Real Tauri AppDir, media-only staging, product not executed',
        'desktopDynamicLibraries':desktop_added,'gstreamerVersion':'1.28.8','originalProduct':before['usr/bin/MewuAI'],
        'stagedProduct':after['usr/bin/MewuAI'],'removed':removed,'mediaReplacements':replaced,
        'gstreamerFamilyProviders':gst_providers,'runtimeProviders':{e['soname']:providers.get(e['soname'],'host-platform-provider') for e in manifest['libraries']},
        'sourceManifest':manifest['sourceManifest'],'elfCount':len(after)}
    assert not receipt.exists();receipt.parent.mkdir(parents=True,exist_ok=True)
    receipt.write_text(json.dumps(evidence,indent=2)+'\n',encoding='utf-8')
    print(json.dumps({'productSha256':after['usr/bin/MewuAI']['sha256'],'gstreamerVersion':'1.28.8','elfCount':len(after),'removed':len(removed),'productExecuted':False}))

if __name__=='__main__':
    p=argparse.ArgumentParser()
    for name in ('appdir','runtime','destination','receipt'):p.add_argument('--'+name,type=pathlib.Path,required=True)
    a=p.parse_args();stage(a.appdir,a.runtime,a.destination,a.receipt)
