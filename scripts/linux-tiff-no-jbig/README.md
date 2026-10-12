# Linux TIFF dependency preparation

This packet builds a replacement `libtiff.so.5` from the authenticated Ubuntu
Jammy source package `4.3.0-6ubuntu0.13`. The four source-file SHA256 values are
locked in `acquire_source.py`; all 48 Ubuntu patches are applied. If the current
authenticated source version changes, the build stops for an explicit profile
review. It never falls back to a bare upstream release archive.

JBIG is disabled through libtiff's supported `jbig=OFF` option. JPEG, old JPEG,
zlib, libdeflate, LZMA, ZSTD, WebP and the internal codecs remain enabled. The
library retains its natural SONAME `libtiff.so.5`, GNU symbol versions and
`$ORIGIN` runpath. The codec-specific `TIFFInitJBIG` export is intentionally
absent. Staging rejects any input ELF that imports it, or an input whose other
TIFF exports, symbol versions or codec dependencies are incompatible.

The build also backports the core one-line change from upstream
[commit 8a4f6b58](https://gitlab.com/libtiff/libtiff/-/commit/8a4f6b587be4fa7bb39fe17f5f9dec52182ab26e),
which fixes valid width-32 alternating fax3 images. The patched Ubuntu source
file's before-hash, after-hash and generated patch-hash are recorded. All Ubuntu
security patches remain in place. This library uses the libtiff BSD-style
license in its exact `COPYRIGHT`; it is not reclassified as LGPL.

## Build and verify

Run from the repository root. Compilation and native tests run inside complete
Docker builds. There is no host compiler requirement and no app/package input.

```text
docker build --target verified -t mewu-linux-tiff-no-jbig:verified scripts/linux-tiff-no-jbig
docker build --target library-artifact --output type=local,dest=<new-artifact-directory> scripts/linux-tiff-no-jbig
```

The `verified` target checks all 19 retained codec factories, writes and decodes
nine RGB TIFF compression types plus Group 3/4 bilevel TIFF, and checks decoded
pixels through the native gdk-pixbuf TIFF loader. It also verifies PNG/JPEG
read-write and a known-color GTK PNG capture under Xvfb. Its process maps must
exclude JBIG, GPM and S-Lang. These are library/loader fixtures, not application
desktop acceptance or production recording tests.

The `library-artifact` target exports only the library, its copyright, locked
source provenance, the explicit upstream backport and `library.json`. Export
depends on the `verified` stage, so a native fixture failure blocks it. Builder
tools and system libraries are excluded from the exported artifact.

## Stage an existing AppDir

`stage_closure.py` requires Python 3 and pyelftools, which the builder image
provides. It takes an existing AppDir and the exported artifact directory and
creates a **new** AppDir; it never runs or modifies the input application.
The receipt must also be a new file outside both inputs and the staged AppDir;
neither the destination nor the receipt may alter the artifact input directory.

```text
python3 scripts/linux-tiff-no-jbig/stage_closure.py --appdir <input-AppDir> --artifact-directory <artifact-directory> --destination <new-AppDir> --receipt <new-receipt.json>
```

The tool replaces the existing TIFF provider after checking SONAME, symbols,
symbol versions, dependency compatibility and the artifact hash. A bundled TIFF
loader remains byte-identical. Every other retained ELF must also remain
byte-identical.

It explicitly omits these dependency chains and reports exact input hashes:

- `libjbig.so.0`, now unnecessary because the replacement TIFF disables JBIG.
- `libgpm.so.2` and `libslang.so.2`, plus the dependent `libaa.so.1`,
  `libcaca.so.0`, `libgstaasink.so` and `libgstcacasink.so`.
- Aliases and the documentation directories of the removed libraries.

The second change deliberately removes ASCII-art `aasink`, `cacasink` and
`cacatv` output. It retains TIFF decoding and the GTK/display dependency paths.
If any remaining ELF needs an excluded SONAME, staging fails. Other license
components still need their own review; this tool does not claim blanket
license compliance or successful installer, application GUI or recording
acceptance.

The packet does not alter the common workflow, Tauri packaging entry point or
production recording feature. Its owner can review and integrate it before
constructing and testing a new AppImage.
