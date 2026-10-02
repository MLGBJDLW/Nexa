// Trusted quality-test runner. Filesystem permissions and these guards prevent
// accidental acceptance shortcuts; this is not an adversarial JavaScript VM.
import assert from 'node:assert';
import strictAssert from 'node:assert/strict';
import { syncBuiltinESMExports } from 'node:module';
import { pathToFileURL } from 'node:url';

// Pin the assertion surface before importing code produced by the agent. Both
// imports share mutable Node builtin objects unless frozen here.
const seen = new Set();
function freezeAssertionSurface(value) {
  if ((typeof value !== 'object' && typeof value !== 'function') || value === null || seen.has(value)) return;
  seen.add(value);
  for (const descriptor of Object.values(Object.getOwnPropertyDescriptors(value))) {
    if ('value' in descriptor) freezeAssertionSurface(descriptor.value);
  }
  Object.freeze(value);
}
freezeAssertionSurface(assert);
freezeAssertionSurface(strictAssert);
syncBuiltinESMExports();

// Rust replaces this literal in an outer temporary file, outside the agent's
// authorized workspace. Keep writing and completion ownership in this closure.
const markCompleted = (() => {
  const write = process.stdout.write.bind(process.stdout);
  const marker = __NEXA_ORACLE_COMPLETION_JSON__;
  return () => write(`\n${marker}\n`);
})();

await import(pathToFileURL(process.argv[4]).href);
markCompleted();
