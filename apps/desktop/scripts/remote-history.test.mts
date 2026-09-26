import assert from 'node:assert/strict';
import test from 'node:test';
import { mergeRemoteHistory } from '../src/features/remote/remoteHistory.ts';

const message = (id: number, content = 'full answer', totalChars = 11) => ({ id: String(id), sortOrder: id, content, totalChars });

test('refresh preserves older pages and expanded text, replaces changed canonical answers', () => {
  const current = [message(0), message(1), message(2), message(3)];
  const incoming = [message(2, 'full'), message(3, 'new answer', 10), message(4)];
  const result = mergeRemoteHistory(current, incoming);
  assert.deepEqual(result.map(item => item.id), ['0', '1', '2', '3', '4']);
  assert.equal(result[2], current[2]);
  assert.equal(result[3], incoming[1]);
  assert.equal(result[4], incoming[2]);
  assert.deepEqual(mergeRemoteHistory(current, []), current);
  assert.deepEqual(mergeRemoteHistory([], incoming), incoming);
});

test('same-length edited content replaces an expanded copy', () => {
  const current = [message(1, 'old answer', 10)];
  const incoming = [message(1, 'new answer', 10)];
  assert.equal(mergeRemoteHistory(current, incoming)[0], incoming[0]);
});

test('history merge indexes each current id once instead of scanning for every incoming record', () => {
  let reads = 0;
  const current = Array.from({ length: 10000 }, (_, id) => ({ ...message(id), get id() { reads += 1; return String(id); } }));
  const incoming = Array.from({ length: 100 }, (_, id) => message(9900 + id));
  assert.equal(mergeRemoteHistory(current, incoming).length, 10000);
  assert.equal(reads, 10000);
});
