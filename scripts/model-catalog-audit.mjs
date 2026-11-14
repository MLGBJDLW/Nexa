import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';
import {
  compareCatalogInventory,
  driftDetected,
  fetchCatalogPages,
  googleCatalogMethod,
  normalizeCatalogModel,
  normalizeGoogleCatalog,
  supportsCatalogOperation,
} from './model-catalog-audit-lib.mjs';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const livePublicCatalogs = process.argv.includes('--live-public');
const outputFlag = process.argv.indexOf('--output');
const outputDir = path.resolve(
  repoRoot,
  outputFlag >= 0 && process.argv[outputFlag + 1]
    ? process.argv[outputFlag + 1]
    : '.artifacts/model-catalog-drift',
);

const sources = [
  ['text', 'shared/provider-presets.json'],
  ['image', 'shared/image-provider-presets.json'],
  ['embedding', 'shared/embedding-provider-presets.json'],
  ['speech_to_text', 'shared/stt-provider-presets.json'],
  ['text_to_speech', 'shared/tts-provider-presets.json'],
  ['decision', 'shared/system-one-provider-presets.json'],
  ['video', 'shared/video-provider-presets.json'],
];

const discoveryCredentials = new Map([
  ['text:openai', 'NEXA_OPENAI_CATALOG_API_KEY'],
  ['text:openrouter', 'NEXA_OPENROUTER_CATALOG_API_KEY'],
  ['text:deepseek', 'NEXA_DEEPSEEK_CATALOG_API_KEY'],
  ['text:alibaba-model-studio', 'NEXA_ALIBABA_CATALOG_API_KEY'],
  ['text:qwen-cloud-intl', 'NEXA_ALIBABA_INTL_CATALOG_API_KEY'],
]);
let scopedCredentials = {};
let credentialConfigurationError = false;
try {
  scopedCredentials = JSON.parse(process.env.NEXA_MODEL_CATALOG_CREDENTIALS || '{}');
  if (!scopedCredentials || Array.isArray(scopedCredentials) || typeof scopedCredentials !== 'object'
    || Object.values(scopedCredentials).some(value => typeof value !== 'string')) {
    credentialConfigurationError = true;
    scopedCredentials = {};
  }
} catch {
  credentialConfigurationError = true;
}

const publicCatalogPaths = new Map([
  ['text:openrouter', '/models'],
  ['image:openrouter-images', '/images/models'],
  ['decision:openrouter', '/models?output_modalities=decisions&endpoints=/api/alpha/decisions'],
  ['embedding:openrouter', '/embeddings/models'],
  ['embedding:jina', '/models'],
  ['video:openrouter-video', '/videos/models'],
]);

const readJson = async (relativePath) => JSON.parse(
  await readFile(path.join(repoRoot, relativePath), 'utf8'),
);

function normalizedUrl(value) {
  return String(value ?? '').trim().replace(/\/+$/, '');
}

function inferredLifecycle(model) {
  const explicit = String(model.status ?? '').toLowerCase();
  if (explicit) return explicit;
  return /preview/i.test(`${model.id} ${model.name} ${model.tagKey ?? ''}`) ? 'preview' : 'active';
}

function inferredAccess(model, lifecycle) {
  if (model.access) return model.access;
  if (lifecycle === 'preview') return 'application';
  if (lifecycle === 'gated') return 'account_enablement';
  return 'public';
}

function inferredReadiness(model) {
  if (model.productReadiness) return model.productReadiness;
  if (model.source === 'discovered') return 'discoverable';
  return 'known';
}

function modelCapabilities(model) {
  return {
    ...(model.capabilities ?? {}),
    ...(Array.isArray(model.inputModalities) ? {
      vision: model.inputModalities.includes('image'),
      audioInput: model.inputModalities.includes('audio'),
      videoInput: model.inputModalities.includes('video'),
    } : {}),
    ...(typeof model.supportsTools === 'boolean' ? { toolCalling: model.supportsTools } : {}),
    ...(typeof model.supportsStructuredOutput === 'boolean'
      ? { structuredOutput: model.supportsStructuredOutput }
      : {}),
    ...(typeof model.supportsDimensionOverride === 'boolean'
      ? { dimensionOverride: model.supportsDimensionOverride }
      : {}),
  };
}

async function buildStaticCatalog() {
  const schema = await readJson('shared/model-catalog.schema.json');
  const lifecycleValues = new Set(schema.$defs.modelLifecycle.enum);
  const accessValues = new Set(schema.$defs.modelAccess.enum);
  const readinessValues = new Set(schema.$defs.productReadiness.enum);
  const errors = credentialConfigurationError ? ['NEXA_MODEL_CATALOG_CREDENTIALS must map endpoint IDs to credential strings'] : [];
  const endpoints = [];
  const endpointIds = new Set();

  for (const [surface, relativePath] of sources) {
    const presets = await readJson(relativePath);
    for (const preset of presets) {
      const endpointId = `${surface}:${preset.id}`;
      if (endpointIds.has(endpointId)) errors.push(`duplicate endpoint id: ${endpointId}`);
      endpointIds.add(endpointId);
      const seenModels = new Set();
      const normalizedModels = (preset.models ?? []).map(model => normalizeCatalogModel(surface, model));
      const removedIds = new Set(normalizedModels
        .filter(model => inferredLifecycle(model) === 'removed')
        .map(model => model.id));
      const models = [];
      for (const model of normalizedModels) {
        if (!model.id || !model.name) errors.push(`${endpointId}: model id and name are required`);
        if (seenModels.has(model.id)) errors.push(`${endpointId}: duplicate model id ${model.id}`);
        seenModels.add(model.id);
        if (removedIds.has(model.replacementModelId)) {
          errors.push(`${endpointId}/${model.id}: replacement ${model.replacementModelId} has been removed`);
        }
        const lifecycle = inferredLifecycle(model);
        const access = inferredAccess(model, lifecycle);
        const productReadiness = inferredReadiness(model);
        if (!lifecycleValues.has(lifecycle)) errors.push(`${endpointId}/${model.id}: invalid lifecycle ${lifecycle}`);
        if (!accessValues.has(access)) errors.push(`${endpointId}/${model.id}: invalid access ${access}`);
        if (!readinessValues.has(productReadiness)) errors.push(`${endpointId}/${model.id}: invalid readiness ${productReadiness}`);
        models.push({
          id: model.id,
          discoveryId: model.resolvedModel ?? model.id,
          aliases: model.aliases ?? [],
          lifecycle,
          deprecationDate: model.deprecationDate ?? null,
          regions: model.regions ?? preset.regions ?? [],
          capabilities: modelCapabilities(model),
          limits: {
            ...(model.contextTokens ? { contextTokens: model.contextTokens } : {}),
            ...(model.maxOutputTokens ? { maxOutputTokens: model.maxOutputTokens } : {}),
          },
          ...(model.inputModalities ? { inputModalities: model.inputModalities } : {}),
          ...(model.outputModalities ? { outputModalities: model.outputModalities } : {}),
          ...(model.supportedParameters ? { supportedParameters: model.supportedParameters } : {}),
        });
      }
      endpoints.push({
        endpointId,
        provider: preset.provider ?? preset.id,
        baseUrl: normalizedUrl(preset.baseUrl),
        apiStyle: preset.apiStyle ?? (surface === 'text' ? 'openai_chat' : ''),
        models,
      });
    }
  }
  return { endpoints, errors };
}

async function probeEndpoint(endpoint) {
  const presetId = endpoint.endpointId.split(':')[1];
  const credentialEnv = discoveryCredentials.get(endpoint.endpointId)
    ?? `NEXA_${presetId.toUpperCase().replace(/[^A-Z0-9]+/g, '_')}_CATALOG_API_KEY`;
  const credential = (scopedCredentials[endpoint.endpointId] ?? process.env[credentialEnv])?.trim() ?? '';
  // A known operation path is also authoritative with credentials. The flag
  // controls anonymous access, not which operation contract gets queried.
  const publicPath = publicCatalogPaths.get(endpoint.endpointId);
  const google = endpoint.provider === 'google' && endpoint.baseUrl.includes('generativelanguage.googleapis.com');
  const anthropic = endpoint.provider === 'anthropic';
  if ((google && !googleCatalogMethod(endpoint)) || !supportsCatalogOperation(endpoint, publicPath)) {
    return { endpointId: endpoint.endpointId, status: 'skipped', reason: 'operation_catalog_not_supported' };
  }
  if (!endpoint.baseUrl || endpoint.baseUrl.includes('example.com')
    || (!publicPath && !google && !anthropic && (!String(endpoint.apiStyle).startsWith('openai')
      || ['vertex', 'bedrock', 'github_copilot_api', 'ollama', 'lm_studio'].includes(endpoint.provider)))) {
    return { endpointId: endpoint.endpointId, status: 'skipped', reason: 'discovery_not_supported' };
  }
  if (!credential && !(livePublicCatalogs && publicPath)) {
    return { endpointId: endpoint.endpointId, status: 'skipped', reason: 'credential_not_configured', credentialEnv };
  }

  try {
    const rows = await fetchCatalogPages(`${endpoint.baseUrl}${publicPath ?? '/models'}`, {
      headers: credential ? google ? { 'x-goog-api-key': credential }
        : anthropic ? { 'x-api-key': credential, 'anthropic-version': '2023-06-01' }
          : { Authorization: `Bearer ${credential}` } : undefined,
    });
    const liveModels = google ? normalizeGoogleCatalog(endpoint, rows) : rows;
    const comparison = compareCatalogInventory({
      ...endpoint,
      operationSpecific: Boolean(publicPath) && endpoint.endpointId !== 'embedding:jina',
    }, liveModels);
    return {
      endpointId: endpoint.endpointId,
      status: 'ok',
      discoveredCount: liveModels.length,
      discoveryScope: credential ? 'credential_catalog' : 'public_catalog',
      unavailableCandidates: comparison.missingIds,
      ...comparison,
    };
  } catch (error) {
    const reason = error instanceof Error && error.name === 'TimeoutError' ? 'timeout'
      : /^http_\d+$|^invalid_pagination$|^invalid_catalog$|^incomplete_catalog$/.test(error?.message) ? error.message : 'request_failed';
    return { endpointId: endpoint.endpointId, status: 'error', reason };
  }
}

function markdownReport(report) {
  const lines = [
    '# Model Catalog Drift Report',
    '',
    `Generated: ${report.generatedAt}`,
    '',
    `Static validation: ${report.staticValidation.errors.length ? 'failed' : 'passed'} (${report.staticValidation.endpointCount} endpoints, ${report.staticValidation.modelCount} model projections)`,
    '',
  ];
  if (report.staticValidation.errors.length) {
    lines.push('## Static validation errors', '', ...report.staticValidation.errors.map((error) => `- ${error}`), '');
  }
  lines.push('## Discovery probes', '');
  for (const probe of report.probes) {
    lines.push(`### ${probe.endpointId}`, '', `Status: ${probe.status}`);
    if (probe.reason) lines.push(`Reason: ${probe.reason}`);
    if (probe.credentialEnv) lines.push(`Credential: ${probe.credentialEnv}`);
    if (probe.status === 'ok') {
      lines.push(
        `Discovered: ${probe.discoveredCount}`,
        `New: ${probe.newIds.length ? probe.newIds.join(', ') : 'none'}`,
        `Missing: ${probe.missingIds.length ? probe.missingIds.join(', ') : 'none'}`,
        `Capability changes: ${probe.capabilityChanged.length ? probe.capabilityChanged.map((item) => `${item.id} (${item.fields.join(', ')})`).join('; ') : 'none'}`,
        `Lifecycle changes: ${probe.lifecycleChanged.length ? probe.lifecycleChanged.map((item) => `${item.id} (${item.curated} → ${item.discovered})`).join('; ') : 'none'}`,
        `Expiration changes: ${probe.expirationChanged.length ? probe.expirationChanged.map(item => `${item.id} (${item.curated ?? 'unspecified'} → ${item.discovered})`).join('; ') : 'none'}`,
        `Region changes: ${probe.regionChanged.length ? probe.regionChanged.map((item) => `${item.id} (${item.curated.join('+')} → ${item.discovered.join('+')})`).join('; ') : 'none'}`,
        `Limit changes: ${probe.limitChanged.length ? probe.limitChanged.map(item => `${item.id} (${item.fields.join(', ')})`).join('; ') : 'none'}`,
        `Modality changes: ${probe.modalityChanged.length ? probe.modalityChanged.map(item => `${item.id} (${item.fields.join(', ')})`).join('; ') : 'none'}`,
        `Parameter changes: ${probe.parameterChanged.length ? probe.parameterChanged.map(item => item.id).join(', ') : 'none'}`,
      );
      lines.push(`Unavailable candidates (not verified retirements): ${probe.unavailableCandidates.join(', ') || 'none'}`);
    }
    lines.push('');
  }
  lines.push(`Discovery coverage: ${report.coverage.probedEndpoints} of ${report.staticValidation.endpointCount} endpoints returned a catalog. Unprobed endpoints are not verified by this report.`, '',
    'New and unavailable IDs are review candidates. Catalog presence alone does not verify inference support; absence alone is not a retirement notice. This audit never rewrites saved models or presets.', '',
    'No authenticated inference was requested. No credentials, request headers, or provider response bodies are stored in this report.', '');
  return lines.join('\n');
}

const staticCatalog = await buildStaticCatalog();
const probes = [];
for (const endpoint of staticCatalog.endpoints) {
  probes.push(await probeEndpoint(endpoint));
}
const modelCount = staticCatalog.endpoints.reduce((total, endpoint) => total + endpoint.models.length, 0);
const completedProbeCount = probes.filter((probe) => probe.status === 'ok').length;
const attemptedProbeCount = probes.filter((probe) => probe.status !== 'skipped').length;
const failedProbeCount = probes.filter((probe) => probe.status === 'error').length;
const allAttemptedProbesSucceeded = attemptedProbeCount > 0 && failedProbeCount === 0;
const hasDrift = staticCatalog.errors.length > 0
  || failedProbeCount > 0
  || probes.some((probe) => probe.status === 'ok' && driftDetected(probe));
const report = {
  schemaVersion: 1,
  generatedAt: new Date().toISOString(),
  hasDrift,
  completedProbeCount,
  attemptedProbeCount,
  failedProbeCount,
  allAttemptedProbesSucceeded,
  coverage: {
    probedEndpoints: completedProbeCount,
    unprobedEndpoints: staticCatalog.endpoints.filter(endpoint => !probes.some(probe => probe.endpointId === endpoint.endpointId && probe.status === 'ok')).map(endpoint => endpoint.endpointId),
    authenticatedInference: false,
  },
  staticValidation: {
    endpointCount: staticCatalog.endpoints.length,
    modelCount,
    errors: staticCatalog.errors,
  },
  probes,
};

await mkdir(outputDir, { recursive: true });
await Promise.all([
  writeFile(path.join(outputDir, 'report.json'), `${JSON.stringify(report, null, 2)}\n`),
  writeFile(path.join(outputDir, 'report.md'), markdownReport(report)),
]);
process.stdout.write(`${JSON.stringify({ outputDir, hasDrift, staticErrors: staticCatalog.errors.length })}\n`);
if (staticCatalog.errors.length) process.exitCode = 1;
