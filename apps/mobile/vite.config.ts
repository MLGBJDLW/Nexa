import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { dedupe: ['react', 'react-dom'], alias: {
    '@': fileURLToPath(new URL('../desktop/src', import.meta.url)),
  } },
  server: { strictPort: true, port: 1422, host: '0.0.0.0', fs: { allow: ['../..'] }, watch: { ignored: ['**/src-tauri/**', '**/target/**'] } },
  build: { target: 'es2022' },
  clearScreen: false,
});
