# Rust dependency compatibility patches

## ZIP security

These four crates retain their previously locked releases. Their
normalized Cargo manifests select ZIP 9 so the application, bundled Copilot CLI,
updater, spreadsheet reader and optional media runtime share the fixed archive
implementation. This avoids unrelated SDK, tensor and updater API migrations.
The small filename API adaptations propagate errors instead of skipping malformed entries. Calamine follows its upstream ZIP 9 adaptation. Upstream licenses are retained beside each crate. Registry release checksums are
recorded in [provenance.json](provenance.json); Cargo.lock pins ZIP 9.0.5 with its
registry checksum.

| Crate | Retained release | Local modification |
| --- | --- | --- |
| calamine | 0.36.0 | ZIP requirement and propagation of ZIP 9 filename decoding errors |
| candle-core | 0.9.2 | ZIP requirement and propagation of ZIP 9 filename decoding errors |
| github-copilot-sdk | 1.0.11 | Build/runtime ZIP requirements and explicit filename-decoding failures |
| tauri-plugin-updater | 2.10.1 | Windows ZIP requirement |

The affected upstream extraction APIs could preserve privileged Unix bits,
create escaping symlinks, or apply permissions through symlinks outside the
extraction root. ZIP 9 fixes these issues. The dependency audit remains enabled;
none of these advisories is suppressed:

- [RUSTSEC-2026-0367](https://rustsec.org/advisories/RUSTSEC-2026-0367)
- [RUSTSEC-2026-0368](https://rustsec.org/advisories/RUSTSEC-2026-0368)
- [RUSTSEC-2026-0369](https://rustsec.org/advisories/RUSTSEC-2026-0369)

The upstream development manifests already adopt ZIP 9, but the published
compatible releases still constrain older majors. Remove each patch when its
normal published dependency accepts a fixed ZIP release and the corresponding
application checks pass. `Cargo.toml.orig` preserves the unmodified release
manifest for comparison. These are source dependencies, not build caches or
additional agent runtimes.

## ONNX build directory compatibility

`ort-sys` retains the locked `2.0.0-rc.12` release and ONNX Runtime 1.24.
Its build script backports the upstream [Cargo 1.100 layout fix](https://github.com/pykeio/ort/commit/eed56816088613b24f8d1a5bf9ae0fd43d8d6a70).
Both `build/ort-sys-<fingerprint>/out` and
`build/ort-sys/<fingerprint>/out` resolve to the actual profile directory;
DLL copying skips optional directories that Cargo has not created. This fixes
cold Windows builds and keeps runtime DLLs beside the application without
changing inference APIs, downloading additional models, or disabling checks.

Only `build/dynamic_link.rs`, `build/main.rs`, and `build/vars.rs` differ from
the registry release. The registry checksum and upstream commit are recorded
in `provenance.json`, and the upstream licenses are retained. Remove this patch
when the application migrates to a published `ort` release containing the fix
(rc.14 or later); those releases also require a manual environment API migration.
