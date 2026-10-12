# SPDX-License-Identifier: MPL-2.0
import hashlib
import json
import os
import subprocess
from pathlib import Path
from debian.deb822 import Sources, Dsc
from debian.debian_support import Version

directory = Path('/packet/source')
records = list(Sources.iter_paragraphs(subprocess.check_output(['apt-cache', 'showsrc', 'tiff'], text=True)))
records = [record for record in records if record.get('Package') == 'tiff']
assert records, 'No authenticated Ubuntu tiff source metadata'
latest = max(records, key=lambda record: Version(record['Version']))
version = latest['Version']
assert version == os.environ['TIFF_SOURCE_VERSION'], ('Current authenticated source changed; review and repin explicitly.', version)
expected_sha256 = {
    'tiff_4.3.0.orig.tar.gz': '0e46e5acb087ce7d1ac53cf4f56a09b221537fc86dfc5daaad1c2e89e1b37ac8',
    'tiff_4.3.0.orig.tar.gz.asc': '6e41d0a4c042d2903f28534eb696a16409ccde9aaa2d02d06b5daaabbfb94aa7',
    'tiff_4.3.0-6ubuntu0.13.debian.tar.xz': '5f73e950e9026c687f79294866d003810d7647ebefa0d4c9ba7bc34985cbdf07',
    'tiff_4.3.0-6ubuntu0.13.dsc': '9db6c768cc538e9ae1767fa2a49264d1f18750b32d72d89e4685c500da53580b',
}
subprocess.run(['apt-get', 'source', '--download-only', 'tiff=' + version], cwd=directory, check=True)
checksums = latest['Checksums-Sha256']
files = []
for entry in checksums:
    path = directory / entry['name']
    assert path.is_file(), path
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    assert digest == entry['sha256'] and path.stat().st_size == int(entry['size']), path
    assert expected_sha256[path.name] == digest, path
    files.append({'name': path.name, 'bytes': path.stat().st_size, 'sha256': digest})
dsc = next(directory.glob('*.dsc'))
with dsc.open(encoding='utf-8') as handle:
    parsed_dsc = Dsc(handle)
assert parsed_dsc['Version'] == version
subprocess.run(['dpkg-source', '-x', str(dsc), str(directory / 'patched')], check=True)
patched = directory / 'patched'
patches = (patched / '.pc/applied-patches').read_text(encoding='utf-8').splitlines()
assert len(patches)==48, 'Pinned Ubuntu patch set changed'
patch_records = [{'name': name, 'sha256': hashlib.sha256((patched / 'debian/patches' / name).read_bytes()).hexdigest()} for name in patches]
evidence = {'sourcePackage': 'tiff', 'version': version, 'repositorySelection': 'Latest authenticated jammy/jammy-updates/jammy-security apt source index; no bare upstream archive.',
            'sourceFiles': files, 'ubuntuAppliedPatches': patch_records,
            'sourceCopyrightSha256': hashlib.sha256((patched / 'debian/copyright').read_bytes()).hexdigest(),
            'changelogFirstEntry': (patched / 'debian/changelog').read_text(encoding='utf-8').split('\n\n',1)[0],
            'aptPolicy': subprocess.check_output(['apt-cache', 'policy', 'libtiff5'], text=True)}
Path('/packet/evidence/ubuntu-source.json').write_text(json.dumps(evidence, indent=2) + '\n', encoding='utf-8')
print(json.dumps({'version': version, 'sourceFiles': files, 'patches': len(patches)}, ensure_ascii=False))
