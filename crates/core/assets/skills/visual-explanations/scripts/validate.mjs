#!/usr/bin/env node
// Node 24, the same dependency-free validator used by the frontend.
import { readFileSync } from 'node:fs';
import { parseChatUi } from '../resources/chat-ui.ts';
const result = parseChatUi(readFileSync(process.argv[2] || 0, 'utf8'));
if (!result.document) { process.stderr.write(result.error + '\n'); process.exitCode = 1; }
else process.stdout.write(JSON.stringify({ valid: true, version: result.document.version, title: result.document.title }) + '\n');
