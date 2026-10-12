# SPDX-License-Identifier: MPL-2.0
import ctypes, hashlib, json, pathlib, re, shutil, subprocess
P=pathlib.Path('/opt/mewu-gst-next')
S=pathlib.Path('/opt/mewu-gst-src')
OUT=P/'share/mewu-sdk-audit';OUT.mkdir(parents=True,exist_ok=True)
manifest=json.loads((S/'sources.json').read_text())
shutil.copyfile(S/'sources.json',OUT/'sources.json')
copied=[]
for source in sorted(S.iterdir()):
    if not source.is_dir() or source.name.startswith('build-') or source.name=='verify-keyring':continue
    for license in source.iterdir():
        if not license.is_file() or not re.match(r'^(COPYING|LICENSE|LICENCE|NOTICE)(\.|$)',license.name,re.I):continue
        dest=OUT/'licenses'/source.name/license.name;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(license,dest)
        copied.append({'sourceDirectory':source.name,'name':license.name,'sha256':hashlib.sha256(license.read_bytes()).hexdigest()})
# The official PTP helper statically links Rust's standard library. Cargo
# application inventory does not cover this independently compiled helper.
rust_version=subprocess.check_output(['rustc','--version'],text=True).strip()
rust_sysroot=pathlib.Path(subprocess.check_output(['rustc','--print','sysroot'],text=True).strip())
rust_docs=rust_sysroot/'share/doc/rust'
rust_notice=OUT/'licenses/rust-standard-library';rust_notice.mkdir(parents=True,exist_ok=True)
if (rust_docs/'COPYRIGHT-library.html').is_file():
    shutil.copyfile(rust_docs/'COPYRIGHT-library.html',rust_notice/'COPYRIGHT-library.html')
    shutil.copytree(rust_docs/'licenses',rust_notice/'licenses',dirs_exist_ok=True)
else:
    # Ubuntu's package stores the complete component copyrights in Debian
    # format rather than the rustup-generated library HTML document.
    package_docs=[pathlib.Path('/usr/share/doc/rustc'),*pathlib.Path('/usr/share/doc').glob('libstd-rust-*')]
    found=0
    for doc in package_docs:
        copyright=doc/'copyright'
        if copyright.is_file():
            target=rust_notice/doc.name/'copyright';target.parent.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(copyright,target);found+=1
    assert found>=2,'Missing exact Rust helper standard-library copyright closure'
runtime_roots=list((P/'lib/gstreamer-1.0').glob('*.so'))+[P/'libexec/gstreamer-1.0/gst-plugin-scanner',P/'libexec/gstreamer-1.0/gst-ptp-helper']
# WebKit directly imports additional base libraries, including GstGL. Auditing
# only recording plugins would leave old system Gst libraries in AppDir.
runtime_roots+=list((P/'lib').glob('libgst*.so'))
# PipeWire dlopens native protocol modules and SPA helpers: DT_NEEDED from the
# GStreamer plugin alone does not cover these runtime dependencies.
runtime_roots+=list((P/'lib/pipewire-0.3').rglob('*.so'))+list((P/'lib/spa-0.2').rglob('*.so'))
libraries={};root_details=[]
for root in runtime_roots:
    output=subprocess.check_output(['ldd',str(root)],text=True)
    assert 'not found' not in output,(root,output)
    root_details.append({'file':str(root),'ldd':output})
    for line in output.splitlines():
        match=re.search(r'=> (/[^ ]+)',line)
        if not match:continue
        path=pathlib.Path(match.group(1)).resolve()
        libraries[str(path)]={'file':str(path),'sha256':hashlib.sha256(path.read_bytes()).hexdigest()}
versions=set()
# Defined/exported libc versions are not this ELF's requirements. Inspect
# undefined dynamic imports on the roots as well as their resolved libraries.
for path in [str(root) for root in runtime_roots]+list(libraries):
    symbols=subprocess.check_output(['readelf','--dyn-syms','--wide',path],text=True)
    for line in symbols.splitlines():
        if ' UND ' in line:versions.update(re.findall(r'@GLIBC_([0-9]+\.[0-9]+(?:\.[0-9]+)?)',line))
for path,detail in libraries.items():
    if not path.startswith(str(P)+'/'):
        candidates=[path]
        # Jammy's merged-/usr inode may still be cataloged by dpkg under /lib.
        # Accept an alternate package path only when it resolves to this exact
        # audited ELF inode, never merely on a matching library basename.
        if path.startswith('/usr/lib/') or path.startswith('/usr/bin/'):
            alias=path[4:]
            if pathlib.Path(alias).is_file() and pathlib.Path(alias).resolve()==pathlib.Path(path):candidates.append(alias)
        package=None
        for candidate in candidates:
            ownership=subprocess.run(['dpkg-query','-S',candidate],text=True,capture_output=True)
            if ownership.returncode==0:package=ownership.stdout.strip();break
        if package is None:raise RuntimeError(f'no exact dpkg-owned inode for runtime dependency: {path}')
        detail['dpkgOwner']=package
        package_name=package.rsplit(': ',1)[0]
        binary_package=package_name.split(':',1)[0]
        detail['dpkgVersion']=subprocess.check_output(['dpkg-query','-W','-f=${Version}',package_name],text=True)
        copyright=pathlib.Path('/usr/share/doc')/binary_package/'copyright'
        if copyright.is_file():
            target=OUT/'licenses'/'debian-packages'/binary_package/'copyright';target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(copyright,target)
            detail['copyrightFile']=str(target.relative_to(OUT))
        else:raise RuntimeError(f'missing runtime dependency copyright: {package_name}')
max_glibc=max(versions,key=lambda x:tuple(map(int,x.split('.'))))
assert tuple(map(int,max_glibc.split('.'))) <= (2,35),max_glibc
codec=ctypes.CDLL(str(P/'lib/libavcodec.so'))
codec.avcodec_license.restype=ctypes.c_char_p
codec.avcodec_configuration.restype=ctypes.c_char_p
ffmpeg_license=codec.avcodec_license().decode();ffmpeg_configuration=codec.avcodec_configuration().decode()
assert 'LGPL version 2.1' in ffmpeg_license,ffmpeg_license
assert '--disable-gpl' in ffmpeg_configuration and '--disable-nonfree' in ffmpeg_configuration
for forbidden in ['libfaad','libfdk-aac','libx264','libx265','libpostproc','libjbig','libavcodec.so.58']:
    assert not any(forbidden in pathlib.Path(path).name for path in libraries),forbidden
plugins=subprocess.check_output([str(P/'bin/gst-inspect-1.0')],text=True)
version=subprocess.check_output([str(P/'bin/gst-inspect-1.0'),'--version'],text=True)
assert '1.28.8' in version
report={'helperRustVersion':rust_version,'sdkRuntimeVersion':version,'sources':manifest,'prefix':str(P),'ffmpegLicense':ffmpeg_license,'ffmpegConfiguration':ffmpeg_configuration,'runtimeRoots':root_details,'dynamicLibraries':list(libraries.values()),'maxRequiredGlibcVersion':max_glibc,'sourceLicenseFiles':copied,'registeredPlugins':plugins,'licenseConclusion':'Source and binary dependency provenance plus exact copyright files; no legal conclusion or blanket whole-SDK no-GPL claim. Only explicit runtime roots and their resolved ELF closure audited; actual final AppImage must rerun its own closure audit.'}
(OUT/'closure.json').write_text(json.dumps(report,indent=2))
print(json.dumps({'gstreamer':'1.28.8','ffmpegLicense':ffmpeg_license,'maxRequiredGlibc':max_glibc,'runtimeRoots':len(runtime_roots),'dynamicLibraries':len(libraries),'sourceLicenseFiles':len(copied)}))
