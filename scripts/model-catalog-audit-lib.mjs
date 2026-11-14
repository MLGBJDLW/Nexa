function normalizedValues(values) {
  return [...new Set((Array.isArray(values) ? values : [])
    .map((value) => String(value).trim().toLowerCase())
    .filter(Boolean))].sort();
}

function stringArray(value) {
  return Array.isArray(value) && value.every(item => typeof item === 'string') ? value : undefined;
}

/** Video presets carry operation contracts under their own public schema. */
export function normalizeCatalogModel(surface, model) {
  if (surface !== 'video') return model;
  const status = { ga: 'active', preview: 'preview', deprecated: 'deprecated', retired: 'removed', unverified: 'gated' }[model.releaseStatus];
  return { ...model, id: model.modelId, name: model.displayName, ...(status ? { status } : {}) };
}

function liveLifecycle(model) {
  const explicit = String(model?.lifecycle ?? model?.status ?? '').trim().toLowerCase();
  if (explicit) return explicit;
  return model?.deprecated === true || model?.expiration_date ? 'deprecated' : null;
}

export function compareCapabilities(curated, discovered) {
  if (!discovered || typeof discovered !== 'object') return [];
  return Object.entries(discovered).flatMap(([key, value]) => {
    // `reasoning: null` is an explicit withdrawal derived from a complete
    // supported-parameter list; absent fields still mean unknown.
    if (value == null) return key === 'reasoning' && value === null && curated?.[key] != null ? [key] : [];
    if (!curated || !Object.hasOwn(curated, key) || curated[key] == null) {
      if (value === false || (Array.isArray(value) && !value.length)) return [];
      return [key];
    }
    if (value && typeof value === 'object' && !Array.isArray(value)) {
      return compareCapabilities(curated[key], value).map(field => `${key}.${field}`);
    }
    if (Array.isArray(value)) {
      return JSON.stringify(normalizedValues(curated[key])) === JSON.stringify(normalizedValues(value)) ? [] : [key];
    }
    return JSON.stringify(curated[key]) === JSON.stringify(value) ? [] : [key];
  }).sort();
}

/** Compare one endpoint/credential snapshot without crossing identity scopes. */
export function compareEndpointModels(endpoint, liveModels) {
  const curatedById = new Map(endpoint.models.map((model) => [model.id, model]));
  const selectionIds = model => [model.id, model.discoveryId, ...(Array.isArray(model.aliases) ? model.aliases : [])]
    .filter(Boolean).map(id => String(id).trim().toLowerCase());
  const knownIds = new Set(endpoint.models.flatMap(selectionIds));
  const liveSelections = new Map();
  const liveById = new Map();
  const newIdSet = new Set();
  for (const live of Array.isArray(liveModels) ? liveModels : []) {
    if (typeof live?.id !== 'string' || !live.id.trim()) continue;
    const id = live.id.trim().toLowerCase();
    liveSelections.set(id, live);
    if (!knownIds.has(id)) {
      newIdSet.add(live.id);
    }
  }
  // Multiple supported wire aliases can resolve to one discovery ID. Match
  // each selection independently instead of losing aliases to a Map collision.
  for (const curated of endpoint.models) {
    const live = selectionIds(curated).map(id => liveSelections.get(id)).find(Boolean);
    if (live) liveById.set(curated.id, live);
  }
  const newIds = [...newIdSet].sort();
  const missingIds = [...curatedById.keys()].filter((id) => !liveById.has(id)).sort();
  const capabilityChanged = [];
  const lifecycleChanged = [];
  const expirationChanged = [];
  const regionChanged = [];
  const limitChanged = [];
  const modalityChanged = [];
  const parameterChanged = [];

  for (const [id, curated] of curatedById) {
    const live = liveById.get(id);
    if (!live) continue;
    const changed = compareCapabilities(curated.capabilities, live.capabilities);
    if (changed.length) capabilityChanged.push({ id, fields: changed });
    const limits = compareCapabilities(curated.limits, live.limits);
    if (limits.length) limitChanged.push({ id, fields: limits });
    const modalities = ['inputModalities', 'outputModalities'].filter(field =>
      Array.isArray(curated[field]) && Array.isArray(live[field])
      && JSON.stringify(normalizedValues(curated[field])) !== JSON.stringify(normalizedValues(live[field])));
    if (modalities.length) modalityChanged.push({ id, fields: modalities });
    if (Array.isArray(live.supportedParameters)
      && JSON.stringify(normalizedValues(curated.supportedParameters)) !== JSON.stringify(normalizedValues(live.supportedParameters))) {
      parameterChanged.push({ id, curated: curated.supportedParameters ?? null, discovered: live.supportedParameters });
    }
    const lifecycle = liveLifecycle(live);
    if (lifecycle && lifecycle !== curated.lifecycle) {
      lifecycleChanged.push({ id, curated: curated.lifecycle, discovered: lifecycle });
    }
    if (live.expiration_date && live.expiration_date !== curated.deprecationDate) {
      expirationChanged.push({ id, curated: curated.deprecationDate ?? null, discovered: live.expiration_date });
    }
    if (Array.isArray(live.regions)) {
      const curatedRegions = normalizedValues(curated.regions);
      const discoveredRegions = normalizedValues(live.regions);
      if (JSON.stringify(curatedRegions) !== JSON.stringify(discoveredRegions)) {
        regionChanged.push({ id, curated: curatedRegions, discovered: discoveredRegions });
      }
    }
  }

  return { newIds, missingIds, capabilityChanged, lifecycleChanged, expirationChanged, regionChanged, limitChanged, modalityChanged, parameterChanged };
}

/** Project explicit catalog evidence only; absent metadata is not a negative. */
export function normalizeDiscoveredModel(endpointId, model) {
  const surface = endpointId.split(':')[0];
  const parameters = stringArray(model.supported_parameters);
  const inputModalities = stringArray(model.architecture?.input_modalities ?? model.input_modalities);
  const outputModalities = stringArray(model.architecture?.output_modalities ?? model.output_modalities)
    ?.map(value => value === 'embeddings' ? 'embedding' : value);
  const reasoningParameters = parameters?.some(parameter =>
    parameter.startsWith('reasoning') || ['include_reasoning', 'thinking', 'thinking_budget'].includes(parameter));
  const reasoningEvidence = model.reasoning?.mandatory === true || model.reasoning?.supports_max_tokens === true
    || stringArray(model.reasoning?.supported_efforts)?.length > 0
    || (parameters == null && typeof model.reasoning?.default_enabled === 'boolean');
  const reasoningUnsupported = parameters && !reasoningParameters && !reasoningEvidence;
  const reasoning = (reasoningParameters || reasoningEvidence) && model.reasoning && typeof model.reasoning === 'object' && !Array.isArray(model.reasoning) ? {
    ...(typeof model.reasoning.mandatory === 'boolean' ? { mode: model.reasoning.mandatory ? 'always' : 'optional' } : {}),
    ...(typeof model.reasoning.default_enabled === 'boolean' ? { defaultEnabled: model.reasoning.default_enabled }
      : model.reasoning.mandatory === true ? { defaultEnabled: true } : {}),
    ...(Array.isArray(model.reasoning.supported_efforts) ? { effortLevels: normalizedValues(model.reasoning.supported_efforts) } : {}),
    ...(typeof model.reasoning.default_effort === 'string' ? { defaultEffort: model.reasoning.default_effort } : {}),
    ...(typeof model.reasoning.supports_max_tokens === 'boolean' ? { thinkingBudget: { enabled: model.reasoning.supports_max_tokens } } : {}),
  } : reasoningUnsupported ? null : undefined;
  return {
    ...model,
    id: endpointId === 'embedding:jina' ? model.id?.replace(/^jina-ai\//, '') : model.id,
    ...(inputModalities ? { inputModalities } : {}),
    ...(outputModalities ? { outputModalities } : {}),
    ...(parameters ? { supportedParameters: parameters } : {}),
    capabilities: {
      ...(model.capabilities ?? {}),
      ...(parameters ? { toolCalling: parameters.includes('tools'), structuredOutput: parameters.includes('structured_outputs') || parameters.includes('response_format') } : {}),
      ...(inputModalities ? { vision: inputModalities.includes('image'), audioInput: inputModalities.includes('audio'), videoInput: inputModalities.includes('video') } : {}),
      ...(reasoning !== undefined ? { reasoning } : {}),
    },
    limits: {
      ...(model.limits ?? {}),
      ...(model.context_length ? { contextTokens: model.context_length } : {}),
      ...(surface === 'text' && model.top_provider?.max_completion_tokens ? { maxOutputTokens: model.top_provider.max_completion_tokens } : {}),
    },
  };
}

/** Gemini model-list methods identify the operation, not just the vendor. */
export function normalizeGoogleCatalog(endpoint, rows) {
  const surface = endpoint.endpointId.split(':')[0];
  const method = surface === 'embedding' ? 'embedContent' : 'generateContent';
  return rows.filter(model => stringArray(model.supportedGenerationMethods)?.includes(method))
    .map(model => ({
      ...model,
      id: String(model.name ?? model.id).replace(/^models\//, ''),
      ...(Number.isSafeInteger(model.inputTokenLimit) && model.inputTokenLimit > 0 ? { context_length: model.inputTokenLimit } : {}),
      ...(Number.isSafeInteger(model.outputTokenLimit) && model.outputTokenLimit > 0 ? { top_provider: { max_completion_tokens: model.outputTokenLimit } } : {}),
      ...(surface === 'embedding' ? { output_modalities: ['embeddings'] } : {}),
    }));
}

/** New IDs are review candidates, not automatic support or retirement claims. */
export function compareCatalogInventory(endpoint, rows) {
  const surface = endpoint.endpointId.split(':')[0];
  const expectedModality = { text: 'text', image: 'image', embedding: 'embeddings', decision: 'decisions', video: 'video', speech_to_text: 'transcription', text_to_speech: 'speech' }[surface];
  const live = rows.filter(model => {
    // ColBERT emits token-level multi-vectors; Nexa's embedding operation
    // consumes one dense vector per input. Catalog presence cannot bridge it.
    if (endpoint.endpointId === 'embedding:jina' && model.id?.replace(/^jina-ai\//, '') === 'jina-colbert-v2') return false;
    const outputs = stringArray(model.architecture?.output_modalities ?? model.output_modalities);
    if (outputs && !outputs.includes(expectedModality)) return false;
    if (surface === 'text' && (model.id?.endsWith(':batch')
      || (Array.isArray(model.endpoints) && !model.endpoints.includes('/api/v1/chat/completions')))) return false;
    return true;
  }).map(model => normalizeDiscoveredModel(endpoint.endpointId, model));
  const active = endpoint.models.filter(model => model.lifecycle !== 'removed');
  const comparison = compareEndpointModels({ ...endpoint, models: active }, live);
  if (surface === 'embedding') {
    // The current embedding adapter deliberately sends text only. Extra vendor
    // input types do not change its wire contract; withdrawals still matter.
    const liveById = new Map(live.map(model => [model.id.toLowerCase(), model]));
    comparison.capabilityChanged = comparison.capabilityChanged.map(change => ({
      ...change,
      fields: change.fields.filter(field => !['vision', 'audioInput', 'videoInput'].includes(field)
        || liveById.get(change.id.toLowerCase())?.capabilities[field] === false),
    })).filter(change => change.fields.length);
    comparison.modalityChanged = comparison.modalityChanged.map(change => ({
      ...change,
      fields: change.fields.filter(field => field !== 'inputModalities'
        || !liveById.get(change.id.toLowerCase())?.inputModalities?.includes('text')),
    })).filter(change => change.fields.length);
  }
  // Tombstones still suppress re-importing a historical ID, without creating a
  // permanent missing-model alarm for models already deliberately retired.
  const known = new Set(endpoint.models.flatMap(model => [model.id, model.discoveryId, ...(model.aliases ?? [])])
    .filter(Boolean).map(id => id.toLowerCase()));
  comparison.newIds = comparison.newIds.filter(id => !known.has(id.toLowerCase()));
  return comparison;
}

/** Follow catalog-owned pagination under one deadline and origin boundary. */
export async function fetchCatalogPages(initialUrl, { headers, fetchImpl = fetch } = {}) {
  const origin = new URL(initialUrl).origin;
  const seen = new Set();
  const rows = [];
  const signal = AbortSignal.timeout(15_000);
  let url = initialUrl;
  let total = null;
  while (url) {
    const parsed = new URL(url);
    if (parsed.origin !== origin || parsed.username || parsed.password || seen.has(parsed.href)) throw new Error('invalid_pagination');
    seen.add(parsed.href);
    const response = await fetchImpl(parsed.href, { headers, signal, redirect: 'error' });
    if (!response.ok) throw new Error(`http_${response.status}`);
    const payload = await response.json();
    const page = payload?.data ?? payload?.models;
    if (!Array.isArray(page) || page.some(model => typeof (model?.id ?? model?.name) !== 'string')) throw new Error('invalid_catalog');
    rows.push(...page);
    if (Number.isInteger(payload.total_count)) total = payload.total_count;
    const next = payload.links?.next;
    if (next) url = new URL(next, parsed).href;
    else if (payload.nextPageToken) {
      parsed.searchParams.set('pageToken', payload.nextPageToken);
      url = parsed.href;
    } else if (payload.has_more) {
      if (!payload.last_id) throw new Error('invalid_pagination');
      parsed.searchParams.set('after_id', payload.last_id);
      url = parsed.href;
    } else url = null;
  }
  if (total != null && rows.length < total) throw new Error('incomplete_catalog');
  return rows;
}

export function driftDetected(comparison) {
  return ['newIds', 'missingIds', 'capabilityChanged', 'lifecycleChanged', 'expirationChanged', 'regionChanged', 'limitChanged', 'modalityChanged', 'parameterChanged']
    .some(field => comparison[field]?.length > 0);
}
