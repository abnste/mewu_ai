# GLib 0.18 compatibility patch

The GTK3 dependency chain uses GLib 0.18.5. [RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html) describes undefined behavior in the string variant iterator: a variadic GLib C function writes an output pointer through an immutable Rust reference. Optimized builds can then dereference a null pointer. A debug build alone does not establish correctness.

The workspace patches `glib` to the exact published 0.18.5 crate in `vendor/glib-0.18.5`. The crate retains its version, MIT license and copyright. The only upstream source modifications are `let p` to `let mut p` and `&p` to `&mut p` in `VariantStrIter::impl_get`, matching [upstream PR 1343](https://github.com/gtk-rs/gtk-rs-core/pull/1343). The official 0.18 branch at `42b9caf98e03ded086362d9653ca58fe94dc8658` still contains the original code.

The original archive SHA256 is `233daaf6e83ae6a12a52055f568f9d7cf4671dabb78ff9560ab6da230ce00ee5`, checked against crates.io release metadata. [The provenance file](glib-backport.json) records every archive file hash and the patched source hash. It contains 121 files; the registry extraction cache's generated marker is not part of the published crate.

```sh
node scripts/check-glib-backport.mjs
cargo test -p mewu-desktop --locked --release --test glib_variant_iter -- --nocapture
```

The Linux regression target exercises real GLib `next`, `next_back`, `nth`, `nth_back` and `last` calls, including Unicode, empty strings, mixed traversal and exhausted boundaries. Preserve the release-profile log and `GLIB_VARIANT_OPTIMIZED=true` evidence. Until executed on Linux, this is a pending check. Other-platform empty test targets are not Linux passes.

Version-based scanners may continue to report 0.18.5. Do not suppress the advisory or claim a clean scan: retain the finding together with verified source provenance and optimized native regression evidence. This patch addresses this advisory only and does not establish that all GTK dependencies are maintained or free of other issues.
