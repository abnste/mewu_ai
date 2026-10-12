# SPDX-License-Identifier: MPL-2.0
"""Create a new AppDir with the locked TIFF library and a checked ELF closure."""
import argparse
import hashlib
import json
import os
import shutil
import stat
from pathlib import Path
from elftools.elf.elffile import ELFFile

EXCLUDED_SONAMES={'libjbig.so.0','libgpm.so.2','libslang.so.2','libaa.so.1','libcaca.so.0'}
EXCLUDED_PLUGINS={'libgstaasink.so','libgstcacasink.so'}
EXCLUDED_DOCS=('libjbig0','libgpm2','libslang2','libaa1','libcaca0')

def elf_record(path):
    with path.open('rb') as handle:
        elf=ELFFile(handle)
        dynamic=elf.get_section_by_name('.dynamic')
        tags=list(dynamic.iter_tags()) if dynamic else []
        symbols=elf.get_section_by_name('.dynsym')
        version=elf.get_section_by_name('.gnu.version_d')
        versions=[]
        if version:
            for _,auxiliaries in version.iter_versions():
                versions.extend(auxiliary.name for auxiliary in auxiliaries)
        return {'bytes':path.stat().st_size,'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),
                'needed':[tag.needed for tag in tags if tag.entry.d_tag=='DT_NEEDED'],
                'soname':[tag.soname for tag in tags if tag.entry.d_tag=='DT_SONAME'],
                'runpath':[tag.runpath for tag in tags if tag.entry.d_tag=='DT_RUNPATH'],
                'definedSymbols':sorted({symbol.name for symbol in symbols.iter_symbols() if symbol.name and symbol['st_shndx']!='SHN_UNDEF'}) if symbols else [],
                'undefinedSymbols':sorted({symbol.name for symbol in symbols.iter_symbols() if symbol.name and symbol['st_shndx']=='SHN_UNDEF'}) if symbols else [],
                'symbolVersions':versions}

def scan(directory):
    result={}
    for folder,_,names in os.walk(directory,followlinks=False):
        for name in names:
            path=Path(folder)/name
            if not stat.S_ISREG(path.lstat().st_mode):
                continue
            with path.open('rb') as handle:
                if handle.read(4)!=b'\x7fELF':
                    continue
            result[path.relative_to(directory).as_posix()]=elf_record(path)
    return result

def stage(appdir,artifacts,destination,receipt):
    assert not receipt.exists() and not receipt.is_symlink(), 'Receipt must be a new file'
    appdir=appdir.resolve()
    artifacts=artifacts.resolve()
    destination=destination.resolve()
    receipt=receipt.resolve()
    assert appdir.is_dir() and artifacts.is_dir()
    assert not destination.exists(), 'Destination must be a new review directory'
    assert not destination.is_relative_to(appdir), 'Destination must be outside input AppDir'
    assert not destination.is_relative_to(artifacts), 'Destination must be outside artifact input'
    assert not receipt.is_relative_to(appdir), 'Receipt must be outside input AppDir'
    assert not receipt.is_relative_to(destination), 'Receipt must be outside staged AppDir'
    assert not receipt.is_relative_to(artifacts), 'Receipt must be outside artifact input'
    manifest=json.loads((artifacts/'library.json').read_text(encoding='utf-8'))
    name=manifest['libraryName']
    assert Path(name).name==name
    replacement=artifacts/name
    assert replacement.resolve().is_relative_to(artifacts)
    new=elf_record(replacement)
    assert new['sha256']==manifest['sha256'] and new['bytes']==manifest['bytes']
    assert new['soname']==['libtiff.so.5'] and new['runpath']==['$ORIGIN']
    assert 'libjbig.so.0' not in new['needed']
    baseline=scan(appdir)
    tiffs=[path for path,record in baseline.items() if record['soname']==['libtiff.so.5']]
    assert len(tiffs)==1, 'Require exactly one original TIFF provider'
    tiff=tiffs[0]
    old=baseline[tiff]
    removed_symbols=set(old['definedSymbols'])-set(new['definedSymbols'])
    assert removed_symbols<= {'TIFFInitJBIG'},removed_symbols
    assert not any('TIFFInitJBIG' in record['undefinedSymbols'] for record in baseline.values())
    assert set(old['symbolVersions'])<=set(new['symbolVersions'])
    assert set(old['needed'])-{'libjbig.so.0'}==set(new['needed']), 'Non-JBIG codec dependency change requires explicit profile review'
    excluded={path for path,record in baseline.items() if set(record['soname'])&EXCLUDED_SONAMES or Path(path).name in EXCLUDED_PLUGINS}
    assert tiff not in excluded
    shutil.copytree(appdir,destination,symlinks=True)
    targets={(destination/path).resolve() for path in excluded}
    aliases=[]
    for folder,_,names in os.walk(destination,followlinks=False):
        for name in names:
            path=Path(folder)/name
            if path.is_symlink() and path.resolve() in targets:
                aliases.append(path.relative_to(destination).as_posix())
    for name in sorted(excluded)+aliases:
        path=destination/name
        assert path.parent.resolve().is_relative_to(destination)
        path.unlink()
    removed_docs=[]
    for package in EXCLUDED_DOCS:
        path=destination/'usr/share/doc'/package
        if path.exists():
            assert path.resolve().is_relative_to(destination)
            shutil.rmtree(path)
            removed_docs.append(path.relative_to(destination).as_posix())
    target=destination/tiff
    assert target.parent.resolve().is_relative_to(destination) and not target.is_symlink()
    shutil.copy2(replacement,target)
    notices=destination/'usr/share/doc/libtiff5/mewu-no-jbig'
    assert notices.resolve().is_relative_to(destination)
    notices.mkdir(parents=True,exist_ok=True)
    for name in ('COPYRIGHT','ubuntu-source.json','upstream-backport.json','library.json'):
        assert (artifacts/name).resolve().is_relative_to(artifacts)
        assert (notices/name).resolve().is_relative_to(destination)
        shutil.copy2(artifacts/name,notices/name)
    remaining=scan(destination)
    assert len(remaining)==len(baseline)-len(excluded)
    for path,record in remaining.items():
        assert not (set(record['needed'])&EXCLUDED_SONAMES),(path,record['needed'])
        if path!=tiff:
            assert record['sha256']==baseline[path]['sha256'],path
    # A bundled TIFF loader, when present, must remain byte-identical.
    loaders=[path for path in baseline if Path(path).name=='libpixbufloader-tiff.so']
    assert all(path in remaining and remaining[path]['sha256']==baseline[path]['sha256'] for path in loaders)
    result={'scope':'Independent new AppDir dependency staging; input app is never run or modified',
            'sourceVersion':manifest['sourceVersion'],'originalElfCount':len(baseline),'stagedElfCount':len(remaining),
            'removed':[{'path':path,**baseline[path]} for path in sorted(excluded)],'removedAliases':aliases,'removedDocs':removed_docs,
            'replacement':{'path':tiff,**new},'removedSymbols':sorted(removed_symbols),
            'allOtherElfBytesUnchanged':True,'noRemainingDirectNeededToExcludedSonames':True,'preservedTiffLoaders':loaders,
            'capabilityChanges':['JBIG TIFF intentionally unsupported; all19 other locked codec factories retained.',
                                 'ASCII-art aasink/cacasink/cacatv outputs intentionally omitted; this tool does not enable production recording.'],
            'remainingElfs':[{'path':path,**record} for path,record in sorted(remaining.items())],
            'licenseScope':'Specified three GPL dependency chains removed; not a blanket license or application GUI acceptance verdict.'}
    receipt.parent.mkdir(parents=True,exist_ok=True)
    receipt.write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
    print(json.dumps({key:result[key] for key in ('originalElfCount','stagedElfCount','removedAliases','removedDocs','removedSymbols','preservedTiffLoaders','capabilityChanges')}))

if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--appdir',type=Path,required=True)
    parser.add_argument('--artifact-directory',type=Path,required=True)
    parser.add_argument('--destination',type=Path,required=True)
    parser.add_argument('--receipt',type=Path,required=True)
    args=parser.parse_args()
    stage(args.appdir,args.artifact_directory,args.destination,args.receipt)
