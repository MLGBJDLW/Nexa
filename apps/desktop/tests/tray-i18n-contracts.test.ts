// @ts-expect-error The contract runner intentionally omits Node ambient types.
import { existsSync, readFileSync } from 'node:fs';
// @ts-expect-error The contract runner intentionally omits Node ambient types.
import { join } from 'node:path';

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const root = process.cwd();
const locales = ['en', 'zh-CN', 'zh-TW', 'ja', 'ko', 'fr', 'de', 'es', 'pt', 'ru'];
const keys = [
  'showNexa',
  'showCompanion',
  'hideCompanion',
  'lockCompanion',
  'unlockCompanion',
  'resetCompanion',
  'companionSettings',
  'quitNexa',
];

const english = JSON.parse(readFileSync(join(root, 'src/i18n/locales/en/tray.json'), 'utf8')) as Record<string, string>;

for (const locale of locales) {
  const path = join(root, 'src/i18n/locales', locale, 'tray.json');
  assert(existsSync(path), `${locale} must provide tray translations`);
  const values = JSON.parse(readFileSync(path, 'utf8')) as Record<string, unknown>;
  for (const key of keys) {
    assert(typeof values[key] === 'string' && values[key].trim().length > 0, `${locale} tray.${key} must be translated`);
    if (locale !== 'en') {
      assert(values[key] !== english[key], `${locale} tray.${key} must not silently fall back to English`);
    }
  }
}

console.log('ok - all supported locales provide complete tray labels');
