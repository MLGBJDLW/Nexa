# Native Plugin Runtime

Native plugins are the last ecosystem surface Nexa should open. They allow
third-party code, hooks, or UI that cannot be represented by connectors, skills,
workflows, adapters, or capability package metadata.

Status: future runtime design. The current capability manifest validator can
classify `native_plugin` declarations; it does not load third-party native code,
hooks, or UI panels. Declarative theme resources and MCP processes are separate
supported surfaces, described in [Ecosystem architecture](ECOSYSTEM_ARCHITECTURE.md).

## Gate Before Building

Do not start native plugin runtime work until these are stable:

- capability package manifests
- MCP connector lifecycle
- skill package import/export and scanning
- workflow package catalog
- protocol exports, starting with scoped MCP server mode

If a requested extension can be built with one of those surfaces, it should not
be a native plugin.

## Runtime Rules

Native plugins must:

- run isolated from core whenever possible
- declare permissions before activation
- declare host targets such as server, UI, or hook
- be disabled by default unless bundled by Nexa
- register tools, hooks, settings, and UI through generic host interfaces
- include compatibility version constraints
- include tests or validation metadata
- never patch core files
- never register hidden high-risk behavior

If a plugin needs a capability the host does not expose, Nexa should expand the
generic host interface. Do not add plugin-specific logic to core.

## Manifest Direction

This is a proposed native-host format. `compatibility`, `targets`, and `hooks`
are not implemented loader features of the current generic capability manifest.
Any future host must validate a supported application-version range before
activation; no compatibility range is implied by the example.

```yaml
id: example-native-plugin
name: Example Native Plugin
surface: native_plugin
description: Example declaration for a future isolated native host.
version: 1
targets:
  - server
permissions:
  read: true
  write: false
  execute: false
  network: false
  nativeCode: true
tools:
  - example_tool
hooks: []
settingsSurfaces: []
```

Native code permission is valid only for `surface: native_plugin`. The
capability manifest validator rejects native code on safer surfaces.

The current declaration and surface rules live in
[capability_package.rs](../crates/core/src/capability_package.rs) and
[ecosystem.rs](../crates/core/src/ecosystem.rs). Admission by that validator is
not executable-host support.
