# Desktop build glob matching

This directory carries the MIT-licensed runtime files from [picomatch 4.0.7](https://registry.npmjs.org/picomatch/-/picomatch-4.0.7.tgz). The original archive has SHA-512 integrity `sha512-qcJu88Q2IWqJsDD529JKMdwGm/dvInW4HvQnRwiH9JtihJvzGOscDtHE3x1pBKeUOTysQ8kVmLnJ2kJu7yhcGA==`.

In `lib/parse.js`, repeated extglobs containing a star use the parser's existing safe rewrite or its literal fallback. This prevents the compiler from nesting a wildcard quantifier inside an unbounded repeat. It also treats escaped stars inside such groups conservatively. Ordinary globs, directory recursion and fixed alternatives retain upstream behavior. Nexa's desktop build uses the same implementation through Vite, tinyglobby and fdir.

Replace the local package and npm override when upstream publishes a fix for [GHSA-fc5q-39xc-vjpf](https://github.com/advisories/GHSA-fc5q-39xc-vjpf).
