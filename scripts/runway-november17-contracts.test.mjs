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
