# RaTeX Unicode font snapshot adaptation

Source: https://github.com/erweixin/RaTeX/tree/776c1d37bafa3bf445a0ab9377c55fe77f7a0133/crates/ratex-unicode-font

Upstream MIT license and authorship are retained. The standalone manifest resolves
the upstream workspace metadata without changing dependency versions.

Mewu adds a read-only snapshot of fonts already loaded by the renderer. Reading
an identity must not eagerly load every fallback or the roughly 180 MiB macOS
emoji collection for a formula which uses no emoji. Rendering and font discovery
remain upstream implementations. The formula worker records this snapshot after
rendering and retains its existing font size, glyph and pixel validation.
