# SPDX-License-Identifier: MPL-2.0
import hashlib, json, os, pathlib, subprocess, sys, tarfile

ROOT=pathlib.Path('/opt/mewu-gst-src')
PREFIX=pathlib.Path('/opt/mewu-gst-next')
VERSION='1.28.8'
def run(args,cwd=None):
    subprocess.run(args,cwd=cwd,check=True)
def configure(component,flags):
    source=ROOT/f'{component}-{VERSION}'
    build=ROOT/f'build-{component}'
    run(['meson','setup',str(build),str(source),'--prefix='+str(PREFIX),'--libdir=lib','--buildtype=release','--wrap-mode=nofallback','-Dauto_features=disabled','-Dtests=disabled','-Ddoc=disabled',*flags])
    run(['meson','compile','-C',str(build),'-j','2'])
    run(['meson','install','-C',str(build)])
step=sys.argv[1]
if step=='verify':
    manifest=json.loads((ROOT/'sources.json').read_text())
    for source in manifest['sources']:
        path=ROOT/source['file']
        assert hashlib.sha256(path.read_bytes()).hexdigest()==source['sha256'],source['file']
        with tarfile.open(path) as archive:
            for member in archive.getmembers():
                parts=pathlib.PurePosixPath(member.name)
                assert not parts.is_absolute() and '..' not in parts.parts
            archive.extractall(ROOT)
    home=ROOT/'verify-keyring';home.mkdir(mode=0o700)
    key=subprocess.check_output(['gpg','--homedir',str(home),'--with-colons','--import-options','show-only','--import',str(ROOT/'ffmpeg-devel.asc')],text=True)
    assert 'fpr:::::::::FCF986EA15E6E293A5644F10B4322F04D67658D8:' in key
    run(['gpg','--batch','--homedir',str(home),'--import',str(ROOT/'ffmpeg-devel.asc')])
    signature=subprocess.check_output(['gpg','--homedir',str(home),'--batch','--status-fd','1','--verify',str(ROOT/'ffmpeg-7.1.5.tar.xz.asc'),str(ROOT/'ffmpeg-7.1.5.tar.xz')],text=True)
    assert '[GNUPG:] VALIDSIG FCF986EA15E6E293A5644F10B4322F04D67658D8 ' in signature
    (ROOT/'ffmpeg-signature.log').write_text(signature)
elif step=='ffmpeg':
    source=ROOT/'ffmpeg-7.1.5'
    flags=['--prefix='+str(PREFIX),'--disable-everything','--disable-autodetect','--disable-programs','--disable-doc','--disable-debug','--disable-network','--disable-gpl','--disable-nonfree','--disable-static','--enable-shared','--disable-avdevice','--disable-postproc','--enable-avcodec','--enable-avformat','--enable-avfilter','--enable-swscale','--enable-swresample','--enable-decoder=aac,h264','--enable-encoder=aac','--enable-parser=aac,h264','--enable-demuxer=mov','--enable-protocol=file']
    run(['./configure',*flags],source);run(['make','-j2'],source);run(['make','install'],source)
    # FFmpeg uses !CONFIG_GPL=yes for the *disabled* feature. Query the actual
    # GNU make variables instead of substring-matching that inverse marker.
    policy=subprocess.check_output(['make','--no-print-directory','-f','ffbuild/config.mak','--eval=print-policy: ; @printf "%s\\n" "$(CONFIG_GPL)" "$(CONFIG_NONFREE)" "$(CONFIG_VERSION3)"','print-policy'],cwd=source,text=True)
    assert all(value.strip()!='yes' for value in policy.splitlines()),repr(policy)
elif step=='orc':
    source=ROOT/'orc-0.4.44';build=ROOT/'build-orc'
    run(['meson','setup',str(build),str(source),'--prefix='+str(PREFIX),'--libdir=lib','--buildtype=release','--wrap-mode=nofallback','-Dtests=disabled','-Dexamples=disabled','-Dhotdoc=disabled'])
    run(['meson','compile','-C',str(build),'-j','2']);run(['meson','install','-C',str(build)])
elif step=='openh264':
    source=ROOT/'openh264-2.6.0'
    # Upstream SHARED is the linker option (-shared), not a boolean. Its
    # install-shared target builds only the library and installs its headers/pc.
    run(['make','-j2','install-shared','BUILDTYPE=Release','PREFIX='+str(PREFIX)],source)
elif step=='gstreamer':
    configure(step,['-Dtools=enabled','-Dptp-helper=enabled','-Dptp-helper-permissions=none','-Dcheck=enabled','-Dintrospection=disabled','-Dexamples=disabled','-Dnls=disabled'])
elif step=='gst-plugins-base':
    configure(step,['-Dx11=enabled','-Dgl=enabled','-Dgl_api=opengl,gles2','-Dgl_platform=egl,glx','-Dgl_winsys=x11,wayland,egl,surfaceless','-Dapp=enabled','-Daudioconvert=enabled','-Daudioresample=enabled','-Daudiotestsrc=enabled','-Dvideoconvertscale=enabled','-Dvideorate=enabled','-Dvideotestsrc=enabled','-Dtypefind=enabled','-Dplayback=enabled','-Dvolume=enabled','-Dorc=enabled','-Dintrospection=disabled','-Dexamples=disabled','-Dnls=disabled'])
elif step=='gst-plugins-good':
    configure(step,['-Daudioparsers=enabled','-Disomp4=enabled','-Dpulse=enabled','-Dautodetect=enabled','-Dvideocrop=enabled','-Dexamples=disabled','-Dnls=disabled','-Dorc=enabled'])
elif step=='gst-plugins-bad':
    configure(step,['-Dvideoparsers=enabled','-Dopenh264=enabled','-Dvoaacenc=enabled','-Dgpl=disabled','-Dexamples=disabled','-Dnls=disabled','-Dorc=enabled','-Dintrospection=disabled'])
elif step=='gst-libav':
    configure(step,[])
elif step=='pipewire':
    source=ROOT/'pipewire-255541eac34370e312f0c5c8c18e46dc1911352f';build=ROOT/'build-pipewire'
    run(['meson','setup',str(build),str(source),'--prefix='+str(PREFIX),'--libdir=lib','--buildtype=release','--wrap-mode=nofallback','-Dauto_features=disabled','-Dgstreamer=enabled','-Dgstreamer-device-provider=disabled','-Ddocs=disabled','-Dman=disabled','-Dexamples=disabled','-Dtests=disabled','-Dinstalled_tests=disabled','-Dsystemd-user-service=disabled','-Dpipewire-jack=disabled','-Dpipewire-v4l2=disabled','-Dpw-cat=disabled','-Dsession-managers=[]','-Dspa-plugins=enabled','-Dsupport=enabled','-Daudioconvert=enabled','-Daudiomixer=disabled','-Daudiotestsrc=disabled','-Dvideotestsrc=disabled'])
    run(['meson','compile','-C',str(build),'-j','2']);run(['meson','install','-C',str(build)])
else:raise RuntimeError('unknown build phase')
