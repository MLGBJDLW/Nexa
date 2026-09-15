declare const require: (id: string) => unknown;
declare const process: { cwd(): string };

const { readFileSync } = require('fs') as {
  readFileSync(path: string, encoding: string): string;
};
const { join } = require('path') as {
  join(...paths: string[]): string;
};

type TestFn = () => void;

const tests: Array<{ name: string; fn: TestFn }> = [];

function test(name: string, fn: TestFn): void {
  tests.push({ name, fn });
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function source(relativePath: string): string {
  return readFileSync(join(process.cwd(), relativePath), 'utf8');
}

test('conversation deletion runs on the dedicated database writer lane', () => {
  const commands = source('src-tauri/src/commands/conversation.rs');
  for (const command of [
    'delete_conversation_cmd',
    'delete_conversations_batch_cmd',
    'delete_all_conversations_cmd',
  ]) {
    const start = commands.indexOf(`pub async fn ${command}`);
    assert(start >= 0, `missing ${command}`);
    const end = commands.indexOf('\n#[tauri::command]', start + 1);
    const body = commands.slice(start, end >= 0 ? end : undefined);
    assert(
      body.includes('db_executor') && body.includes('.write('),
      `${command} must not run synchronous cascading SQLite work on an async command worker`,
    );
  }
});


for (const { name, fn } of tests) {
  try {
    fn();
    console.log(`ok - ${name}`);
  } catch (error) {
    console.error(`not ok - ${name}`);
    throw error;
  }
}
