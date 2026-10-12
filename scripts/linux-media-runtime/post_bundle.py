# SPDX-License-Identifier: MPL-2.0
"""Install a new, audited post-bundle AppDir in the disposable build tree.

The original generated directory is moved to a unique sibling for rollback;
no application/user path is accepted and no product is executed.
"""
import argparse, errno, importlib.util, json, os, pathlib, shutil, subprocess, tempfile

HERE=pathlib.Path(__file__).resolve().parent

def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path);value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value

def install(workspace,runtime,tiff):
    workspace=workspace.resolve(strict=True);runtime=runtime.resolve(strict=True);tiff=tiff.resolve(strict=True)
    assert len(str(workspace))<=4096 and workspace.is_dir()
    subprocess.run(['git','-C',str(workspace),'rev-parse','--is-inside-work-tree'],check=True,stdout=subprocess.DEVNULL)
    parent=workspace/'target/release/bundle/appimage';assert parent.resolve(strict=True)==parent
    original=parent/'MewuAI.AppDir';assert original.is_dir() and not original.is_symlink() and original.resolve()==original
    scratch=pathlib.Path(tempfile.mkdtemp(prefix='.mewu-media-stage-',dir=parent))
    assert scratch.parent==parent and scratch.resolve()==scratch
    tiff_stage=module('mewu_tiff_stage',HERE.parent/'linux-tiff-no-jbig/stage_closure.py')
    media_stage=module('mewu_media_stage',HERE/'stage_appdir.py')
    tiff_stage.stage(original,tiff,scratch/'tiff.AppDir',scratch/'tiff.json')
    media_stage.stage(scratch/'tiff.AppDir',runtime,scratch/'final.AppDir',scratch/'media.json')
    # Validate canonical resources and permissions before touching the input.
    script="import {inspectAppDir} from './scripts/normalize-linux-appimage.mjs'; const r=await inspectAppDir(process.argv[1],{normalized:true}); console.log(JSON.stringify({files:r.entries.length,bytes:r.totalBytes}));"
    subprocess.run(['node','--input-type=module','-e',script,str(scratch/'final.AppDir')],cwd=workspace,check=True)
    source=subprocess.check_output(['git','-C',str(workspace),'rev-parse','HEAD'],text=True).strip()
    expected=os.environ.get('MEWU_BUILD_COMMIT',source);assert source==expected
    backup=scratch/'original.AppDir'
    try:original.rename(backup)
    except OSError as error:
        if error.errno!=errno.EXDEV:raise
        # OverlayFS can refuse renaming a directory from a lower image layer
        # even though stat.dev matches. Copy and verify every byte/mode before
        # removing this exact, generated build directory. No arbitrary target.
        identity=original.stat();shutil.copytree(original,backup,symlinks=True)
        comparison="import {inspectAppDir} from './scripts/normalize-linux-appimage.mjs'; const [a,b]=await Promise.all(process.argv.slice(1).map(p=>inspectAppDir(p,{requireResources:false}))); const content=x=>x.entries.map(({relative,kind,target,bytes,sha256,mode})=>({relative,kind,target,bytes,sha256,mode})).sort((a,b)=>a.relative.localeCompare(b.relative)); if(JSON.stringify(content(a))!==JSON.stringify(content(b)))throw Error('Original rollback backup differs');"
        subprocess.run(['node','--input-type=module','-e',comparison,str(original),str(backup)],cwd=workspace,check=True)
        current=original.stat();assert (identity.st_dev,identity.st_ino)==(current.st_dev,current.st_ino)
        assert original.resolve()==parent/'MewuAI.AppDir' and backup.resolve().parent==scratch
        shutil.rmtree(original)
    try:(scratch/'final.AppDir').rename(original)
    except BaseException:
        backup.rename(original);raise
    metadata={'schemaVersion':1,'sourceCommit':source,'appdir':str(original.relative_to(workspace)),
              'mediaReceipt':str((scratch/'media.json').relative_to(workspace)),
              'originalBackup':str(backup.relative_to(workspace)),'applicationExecuted':False}
    print(json.dumps(metadata))

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--workspace',type=pathlib.Path,default=pathlib.Path.cwd())
    p.add_argument('--runtime',type=pathlib.Path,required=True);p.add_argument('--tiff-artifact',type=pathlib.Path,required=True)
    a=p.parse_args();install(a.workspace,a.runtime,a.tiff_artifact)
