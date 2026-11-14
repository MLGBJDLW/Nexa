function normalizedValues(values) {
  return [...new Set((Array.isArray(values) ? values : [])
    .map((value) => String(value).trim().toLowerCase())
    .filter(Boolean))].sort();
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
  return Object.entries(discovered)
    .filter(([key, value]) => Object.hasOwn(curated, key) && curated[key] !== value)
    .map(([key]) => key)
    .sort();
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

  for (const [id, curated] of curatedById) {
    const live = liveById.get(id);
    if (!live) continue;
    const changed = compareCapabilities(curated.capabilities, live.capabilities);
    if (changed.length) capabilityChanged.push({ id, fields: changed });
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

  return { newIds, missingIds, capabilityChanged, lifecycleChanged, expirationChanged, regionChanged };
}

/** Check a small public availability contract without treating an intentionally
 * curated provider catalog as an exhaustive mirror of every live model. */
export function compareRequiredModelIds(requiredIds, liveModels) {
  const liveIds = new Set((Array.isArray(liveModels) ? liveModels : [])
    .map((model) => typeof model?.id === 'string' ? model.id.trim().toLowerCase() : '')
    .filter(Boolean));
  const missingIds = [...new Set((Array.isArray(requiredIds) ? requiredIds : [])
    .map((id) => String(id).trim())
    .filter(Boolean))]
    .filter((id) => !liveIds.has(id.toLowerCase()))
    .sort();
  return {
    newIds: [],
    missingIds,
    capabilityChanged: [],
    lifecycleChanged: [],
    expirationChanged: [],
    regionChanged: [],
  };
}

export function driftDetected(comparison) {
  return comparison.newIds.length > 0
    || comparison.missingIds.length > 0
    || comparison.capabilityChanged.length > 0
    || comparison.lifecycleChanged.length > 0
    || comparison.expirationChanged.length > 0
    || comparison.regionChanged.length > 0;
}

/** Validate one response page; never mistake an error object for an empty catalog.
 * Do not follow arbitrary next URLs with credentials. Incomplete pages remain explicit. */
export function inspectCatalogPage(payload) {
  const rows = Array.isArray(payload?.data) ? payload.data : payload?.models;
  if (!Array.isArray(rows) || rows.some(row => typeof row?.id !== 'string' || !row.id.trim())) {
    throw new Error('invalid_catalog_payload');
  }
  const uniqueCount = new Set(rows.map(row => row.id.trim().toLowerCase())).size;
  if (uniqueCount !== rows.length) throw new Error('duplicate_catalog_ids');
  const total = payload.total_count;
  if (total !== undefined && (!Number.isInteger(total) || total < rows.length)) {
    throw new Error('invalid_catalog_total');
  }
  const next = payload.links?.next ?? payload.next ?? payload.next_cursor;
  const hasNext = Boolean(next) || payload.has_more === true;
  return {
    rows,
    pagination: {
      receivedCount: rows.length,
      uniqueCount,
      totalCount: total ?? null,
      pagesFetched: 1,
      hasNext,
      completeness: hasNext || (total !== undefined && total > uniqueCount)
        ? 'partial' : total === uniqueCount ? 'complete' : 'unknown',
    },
  };
}

/** Inventory candidates are not approved integrations. Preserve unknown metadata
 * from non-OpenRouter catalogs instead of assuming OpenRouter's architecture schema. */
export function inventoryCandidates(endpoint, rows) {
  const accepted = [];
  const excluded = [];
  const unknownMetadataIds = [];
  const modalityCounts = {};
  const isRouter = endpoint.endpointId === 'text:openrouter' || endpoint.endpointId === 'image:openrouter-images';
  const expected = endpoint.endpointId.startsWith('image:') ? 'image' : 'text';
  for (const row of rows) {
    const modalities = row.architecture?.output_modalities ?? row.output_modalities;
    for (const modality of new Set(Array.isArray(modalities) ? modalities : ['unspecified'])) {
      modalityCounts[modality] = (modalityCounts[modality] ?? 0) + 1;
    }
    let reason;
    if (isRouter && row.id.endsWith(':batch')) reason = 'batch_only';
    else if (isRouter && Array.isArray(modalities) && !modalities.includes(expected)) reason = 'other_modality';
    else if (endpoint.endpointId === 'text:openrouter' && Array.isArray(row.endpoints)
      && !row.endpoints.includes('/api/v1/chat/completions')) reason = 'no_chat_endpoint';
    if (reason) excluded.push({ id: row.id, reason });
    else {
      accepted.push(row);
      if (!Array.isArray(modalities)) unknownMetadataIds.push(row.id);
    }
  }
  return { ...compareEndpointModels(endpoint, accepted), receivedCount: rows.length,
    eligibleCount: accepted.length, excluded, unknownMetadataIds, modalityCounts };
}

/** Keep availability contracts distinct from advisory inventory review. */
export function summarizeDiscovery(probes) {
  const inventoryCandidateCount = probes.reduce((sum, p) => sum + (p.inventory?.newIds.length ?? 0), 0);
  const incompleteCatalogEndpoints = probes.filter(p => p.status === 'ok'
    && p.pagination?.completeness !== 'complete').map(p => p.endpointId);
  const unassessedInventoryEndpoints = probes.filter(p => p.status !== 'ok').map(p => p.endpointId);
  const observedCatalogCount = probes.filter(p => p.status === 'ok').length;
  return {
    inventoryStatus: probes.length === 0 ? 'not_requested' : observedCatalogCount === 0 ? 'unavailable' : 'observed_catalogs_only',
    observedCatalogCount,
    unassessedInventoryEndpoints,
    inventoryCandidateCount,
    hasInventoryCandidates: inventoryCandidateCount > 0,
    incompleteCatalogEndpoints,
    requiresInventoryReview: inventoryCandidateCount > 0 || incompleteCatalogEndpoints.length > 0 || unassessedInventoryEndpoints.length > 0,
  };
}
