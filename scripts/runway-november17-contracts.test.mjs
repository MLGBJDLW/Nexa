import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
const read = file => JSON.parse(readFileSync(new URL(`../${file}`, import.meta.url), 'utf8'));
const provider = read('shared/video-provider-presets.json').find(p => p.id === 'runway-global');
const snapshot = read('crates/core/src/media_generation/adapters/fixtures/runway-video-contracts.json');
for (const [id, maximum, promptMax] of [['kling_4_flash', 20, 11000], ['gemini_omni_flash_1.2', 15, 4000]]) {
  test(`${id}: bounded opt-in text contract matches official schema without inferred media support`, () => {
    const model = provider.models.find(m => m.modelId === id);
    assert.equal(model.selectable, true);
    assert.equal(model.pricing.kind, 'unavailable');
    assert.deepEqual(model.operationCapabilities.map(o => o.operation), ['text_to_video']);
    const op = model.operationCapabilities[0];
    const wire = op.wireContract;
    assert.deepEqual(op.inputRoles, []);
    assert.equal(op.supportsAudio, false);
    assert.equal(op.supportsSeed, false);
    assert.equal(wire.promptMax, promptMax);
    assert.equal(wire.promptRequired, true);
    assert.equal(wire.maxReferenceImages + wire.maxReferenceVideos + wire.maxReferenceAudio, 0);
    const contract = snapshot.contracts.find(c => c.model === id && c.path === '/v1/text_to_video');
    assert.equal(contract.checkedAt, '2026-11-17');
    assert.match(contract.sourceSha256, /^[a-f0-9]{64}$/);
    assert.ok(wire.promptMax <= contract.fields.promptText.maxLength);
    const duration = contract.fields.duration.anyOf?.find(s => s.type === 'integer') ?? contract.fields.duration;
    for (const option of op.durationOptions) {
      assert.equal(option.minDurationSeconds, duration.minimum);
      assert.equal(option.maxDurationSeconds, maximum);
      assert.equal(maximum, duration.maximum);
      for (const ratio of Object.values(wire.ratios[option.resolution])) {
        assert.ok(contract.fields.ratio.enum.includes(ratio));
      }
    }
    assert.equal(wire.videoField, undefined);
    assert.equal(wire.videoMode, undefined);
    assert.equal(wire.durationField, true);
  });
}
test('Kling Flash never inherits full Kling 4 ratios or duration and Gemini preserves all verified output sizes', () => {
  const kling = provider.models.find(m => m.modelId === 'kling_4_flash').operationCapabilities[0];
  assert.deepEqual(Object.keys(kling.wireContract.ratios), ['720P']);
  assert.equal(kling.wireContract.ratios['720P']['1:1'], '960:960');
  assert.equal(kling.wireContract.ratios['720P']['21:9'], '1464:628');
  const gemini = provider.models.find(m => m.modelId === 'gemini_omni_flash_1.2').operationCapabilities[0];
  assert.deepEqual(Object.keys(gemini.wireContract.ratios), ['360P', '720P', '1080P', '4K']);
});

test('Seedance 2.5 Flash exposes only documented text video ratios, duration, and output pricing', () => {
  const id = 'seedance2_5_flash';
  const model = provider.models.find(m => m.modelId === id);
  assert.equal(model.selectable, true);
  assert.equal(model.supportsWebhook, false);
  assert.deepEqual(model.operationCapabilities.map(o => o.operation), ['text_to_video']);
  const [op] = model.operationCapabilities;
  const { wireContract: wire } = op;
  assert.deepEqual(op.inputRoles, []);
  assert.equal(op.supportsAudio, true);
  assert.equal(op.supportsSeed, true);
  assert.equal(wire.promptMin, 1);
  assert.equal(wire.promptMax, 15000);
  assert.equal(wire.promptRequired, true);
  assert.equal(wire.maxReferenceImages + wire.maxReferenceVideos + wire.maxReferenceAudio, 0);
  assert.deepEqual(Object.keys(wire.ratios), ['480P', '720P']);
  assert.deepEqual(op.durationOptions.map(o => o.resolution), ['480P', '720P']);
  const contract = snapshot.contracts.find(c => c.model === id && c.path === '/v1/text_to_video');
  assert.equal(contract.checkedAt, '2026-11-21');
  assert.match(contract.sourceSha256, /^[a-f0-9]{64}$/);
  assert.deepEqual(contract.required, ['model']);
  assert.deepEqual(contract.fields.promptText, { type: 'string', minLength: 1, maxLength: 15000 });
  assert.deepEqual(contract.fields.duration.anyOf.find(s => s.type === 'integer'), {
    type: 'integer', minimum: 4, maximum: 30,
  });
  for (const option of op.durationOptions) {
    assert.equal(option.minDurationSeconds, 4);
    assert.equal(option.maxDurationSeconds, 30);
  }
  assert.deepEqual(Object.values(wire.ratios).flatMap(Object.values).sort(), [...contract.fields.ratio.enum].sort());
  assert.equal(wire.ratioField, 'ratio');
  assert.equal(wire.resolutionField, undefined);
  assert.equal(wire.durationField, true);
  assert.equal(wire.videoField, undefined);
  assert.equal(wire.videoMode, undefined);
  assert.equal(model.pricing.kind, 'tiered_per_second');
  assert.equal(model.pricing.currency, 'USD');
  assert.equal(model.pricing.minimumAmountMicros, 520000);
  assert.deepEqual(model.pricing.tiers, [
    { resolution: '480P', microsPerSecond: 130000 },
    { resolution: '720P', microsPerSecond: 200000 },
  ]);
  assert.equal(model.pricing.inputVideoTiers, undefined);
});
