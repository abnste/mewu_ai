# SPDX-License-Identifier: MPL-2.0
"""Build-time EGL pixel check; does not execute Mewu or certify a desktop GUI."""
import argparse, ctypes as C, json, os, pathlib, subprocess, sys

def pixels():
    egl=C.CDLL('libEGL.so.1')
    pointer=C.c_void_p;integer=C.c_int;uint=C.c_uint;ints=C.POINTER(integer)
    def function(name,result,args):
        value=getattr(egl,name);value.restype=result;value.argtypes=args;return value
    error=function('eglGetError',uint,[])
    display=function('eglGetPlatformDisplay',pointer,[uint,pointer,ints])(0x31DD,None,None)
    major=integer();minor=integer()
    assert display and function('eglInitialize',uint,[pointer,ints,ints])(display,C.byref(major),C.byref(minor)),hex(error())
    attributes=(integer*13)(0x3033,1,0x3040,4,0x3024,8,0x3023,8,0x3022,8,0x3021,8,0x3038)
    config=pointer();count=integer()
    assert function('eglChooseConfig',uint,[pointer,ints,C.POINTER(pointer),integer,ints])(display,attributes,C.byref(config),1,C.byref(count)) and count.value==1,hex(error())
    assert function('eglBindAPI',uint,[uint])(0x30A0),hex(error())
    context=function('eglCreateContext',pointer,[pointer,pointer,pointer,ints])(display,config,None,(integer*3)(0x3098,2,0x3038))
    assert context,hex(error())
    surface=function('eglCreatePbufferSurface',pointer,[pointer,pointer,ints])(display,config,(integer*5)(0x3057,16,0x3056,16,0x3038))
    assert surface,hex(error())
    current=function('eglMakeCurrent',uint,[pointer,pointer,pointer,pointer])
    assert current(display,surface,surface,context),hex(error())
    try:
        # Late loading matches Epoxy's lazy dispatch. Merely resolving symbols
        # missed a bundled old GLES frontend returning null/no-op on a new host.
        gles=C.CDLL('libGLESv2.so.2')
        gles.glViewport.argtypes=[integer]*4;gles.glViewport(0,0,16,16)
        gles.glClearColor.argtypes=[C.c_float]*4;gles.glClearColor(.2,.4,.6,1)
        gles.glClear.argtypes=[uint];gles.glClear(0x4000)
        pixel=(C.c_ubyte*4)()
        gles.glReadPixels.argtypes=[integer]*4+[uint,uint,pointer]
        gles.glReadPixels(0,0,1,1,0x1908,0x1401,pixel)
        gles.glGetString.argtypes=[uint];gles.glGetString.restype=C.c_char_p
        version=gles.glGetString(0x1F02);gl_error=gles.glGetError()
        paths=sorted({line.split()[-1] for line in pathlib.Path('/proc/self/maps').read_text().splitlines()
            if any(name in line for name in ('libGLES','libEGL','libGLdispatch','libwayland-client','libX11','libxcb.so'))
            and line.split()[-1].startswith('/')})
        evidence={'eglVersion':[major.value,minor.value],'glVersion':version.decode() if version else None,
                  'pixel':list(pixel),'glError':gl_error,'providerPaths':paths,
                  'nativeGuiVerified':False,'installerVerified':False}
        print(json.dumps(evidence),flush=True)
        assert version and gl_error==0 and all(abs(a-b)<=1 for a,b in zip(pixel,[51,102,153,255])),evidence
    finally:
        current(display,None,None,None)
        function('eglDestroySurface',uint,[pointer,pointer])(display,surface)
        function('eglDestroyContext',uint,[pointer,pointer])(display,context)
        function('eglTerminate',uint,[pointer])(display)

def verify(root):
    from stage_appdir import HOST_GRAPHICS,scan
    root=root.resolve(strict=True)
    assert root.is_dir() and (root/'usr/bin/MewuAI').is_file()
    assert not any(set(entry['soname'])&HOST_GRAPHICS for entry in scan(root).values())
    script=pathlib.Path(__file__).resolve()
    environment=os.environ.copy()
    for name in list(environment):
        if name.startswith('LD_'):environment.pop(name)
    results=[]
    for name in ('system','packaged'):
        case=environment.copy()
        if name=='packaged':case['LD_LIBRARY_PATH']=str(root/'usr/lib')
        run=subprocess.run([sys.executable,str(script),'--pixels'],env=case,text=True,capture_output=True,timeout=30)
        if run.returncode:
            sys.stdout.write(run.stdout);sys.stderr.write(run.stderr)
            raise RuntimeError('Actual EGL pixel verification failed: '+name)
        result=json.loads(run.stdout)
        assert not any(pathlib.Path(p).is_relative_to(root) for p in result['providerPaths'])
        results.append({'case':name,**result})
    assert results[0]['providerPaths']==results[1]['providerPaths'],'Graphics providers differ in packaged environment'
    print(json.dumps({'kind':'host_graphics_pixels','cases':results,'nativeGuiVerified':False,'installerVerified':False}))

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--appdir',type=pathlib.Path);parser.add_argument('--pixels',action='store_true')
    args=parser.parse_args()
    assert args.pixels != bool(args.appdir),'Choose AppDir verification or isolated pixel probe'
    if args.pixels:pixels()
    else:verify(args.appdir)
