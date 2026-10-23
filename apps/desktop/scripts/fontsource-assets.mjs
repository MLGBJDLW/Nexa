import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Keep WOFF2 sources intact; supported WebViews do not need duplicate WOFF/TTF. */
export function omitLegacyFontSources(code) {
  return code.replace(/\bsrc\s*:[^;{}]+(?=;|})/g, (declaration) => {
    if (!/url\([^)]*\.woff2\)\s*format\(['"]woff2['"]\)/.test(declaration)) return declaration;
    return declaration.replace(/,\s*url\([^)]*\.(?:woff|ttf)\)\s*format\(['"](?:woff|truetype)['"]\)/g, '');
  });
}

/** Keep the original licenses beside offline assets and omit legacy font duplicates. */
export function fontsourceAssets() {
  const root = fileURLToPath(new URL('../', import.meta.url));
  return {
    name: 'nexa-fontsource-assets',
    enforce: 'pre',
    transform(code, id) {
      const fontStylesheet = id.includes('@fontsource') || /[/\\]katex[/\\]dist[/\\]katex(?:\.min)?\.css$/.test(id);
      if (!fontStylesheet || !id.endsWith('.css')) return null;
      return omitLegacyFontSources(code);
    },
    generateBundle() {
      const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
      for (const name of Object.keys(pkg.dependencies).filter(name => name.startsWith('@fontsource') || name === 'katex')) {
        this.emitFile({ type: 'asset', fileName: `font-licenses/${name.replace('@', '').replace('/', '-')}.txt`, source: readFileSync(join(root, 'node_modules', name, 'LICENSE'), 'utf8') });
      }
    },
  };
}
