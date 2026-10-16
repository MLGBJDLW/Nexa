# Bundled shell utilities

`npm run shell-tools:bundle` stages pinned, SHA-256-verified upstream `rg` and
`jq` executables here. Windows bundles also contain Linux executables for WSL.
Generated files are not committed. Licenses ship beside each executable.

Nexa adds these tools only to its child-process environments. It does not edit
system PATH, shell startup files, or install packages in WSL distributions.
