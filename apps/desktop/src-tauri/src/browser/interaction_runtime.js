(() => {
  if (window.__NEXA_BROWSER_RUNTIME__) return;
  let pointerAppearance = __NEXA_POINTER_APPEARANCE__;
  // These are independent WebViews, so application chrome styles do not reach
  // their documents. Keep native drag targets visible without changing any
  // page-owned overflow rules (including modal and nested-scroll locks).
  const scrollbarCss = `
    :root, :root * { scrollbar-width: auto !important; scrollbar-color: auto !important; }
    :root::-webkit-scrollbar, :root *::-webkit-scrollbar {
      display: block !important; width: 12px !important; height: 12px !important;
    }
    :root::-webkit-scrollbar-track, :root *::-webkit-scrollbar-track {
      background: rgba(128, 128, 128, .12) !important;
    }
    :root::-webkit-scrollbar-thumb, :root *::-webkit-scrollbar-thumb {
      background: rgba(128, 128, 128, .65) !important;
      background-clip: padding-box !important; border: 2px solid transparent !important;
      border-radius: 6px !important; min-width: 24px !important; min-height: 24px !important;
    }
    :root::-webkit-scrollbar-thumb:hover, :root *::-webkit-scrollbar-thumb:hover {
      background-color: rgba(128, 128, 128, .9) !important;
    }
  `;
  try {
    // A constructed sheet also works on documents that disallow inline styles.
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(scrollbarCss);
    document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
  } catch (_) {
    const install = () => {
      const style = document.createElement('style');
      style.textContent = scrollbarCss;
      (document.head || document.documentElement).append(style);
    };
    if (document.documentElement) install();
    else document.addEventListener('DOMContentLoaded', install, { once: true });
  }
  const runtime = {
    refs: new Map(),
    refIds: new WeakMap(),
    nextRefId: 0,
    userEpoch: 0,
    synthetic: false,
    pickMode: null,
    pendingArtifact: null,
    overlay: null,
    regionStart: null,
    agentCursor: null,
    cursorDocument: null,
    cursorPoint: null,
    observationOptions: { query: '', offset: 0 },
    geometryIds: new WeakMap(),
    nextGeometryId: 0,
  };

  const labelledByText = (el) => (el.getAttribute?.('aria-labelledby') || '').split(/\s+/).filter(Boolean).map(id => {
    const label = el.getRootNode().getElementById?.(id) || el.ownerDocument.getElementById(id);
    return label?.textContent || '';
  }).join(' ').replace(/\s+/g, ' ').trim();
  const textOf = (el) => String(
    labelledByText(el) || el.getAttribute?.('aria-label') || Array.from(el.labels || []).map(label => label.innerText).join(' ') || el.innerText || el.getAttribute?.('alt') || (['button','submit','reset'].includes(el.type) ? el.value : '') || el.getAttribute?.('placeholder') || el.getAttribute?.('title') || el.getAttribute?.('name') || ''
  ).trim().slice(0, 240);
  const valueOf = (el) => {
    if (el.tagName === 'INPUT' && ['password', 'hidden', 'file', 'checkbox', 'radio', 'button', 'submit', 'reset', 'image'].includes(el.type)) return null;
    if (['current-password', 'new-password'].includes(el.autocomplete)) return null;
    if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') return String(el.value);
    if (el.isContentEditable) return el.innerText || '';
    return null;
  };
  const roleOf = (el) => el.getAttribute?.('role') || (el.tagName === 'INPUT' && ({ checkbox: 'checkbox', radio: 'radio', range: 'slider', button: 'button', submit: 'button', reset: 'button' }[el.type])) || ({
    A: 'link', BUTTON: 'button', INPUT: 'textbox', TEXTAREA: 'textbox', SELECT: 'combobox'
  }[el.tagName] || '');
  const composedParent = (element) => element.assignedSlot || element.parentElement || element.getRootNode?.().host;
  const deepElementFromPoint = (root, x, y) => {
    // Start at the document so a page-level overlay still blocks the target.
    // Querying only the target's shadow root would bypass that occlusion check.
    let hit = root.elementFromPoint(x, y);
    while (hit?.shadowRoot) {
      const inner = hit.shadowRoot.elementFromPoint(x, y);
      if (!inner || inner === hit) break;
      hit = inner;
    }
    return hit;
  };
  const deepActiveElement = (root) => {
    let active = root.activeElement;
    while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
    return active;
  };
  const composedContains = (target, node) => {
    for (let current = node; current; current = composedParent(current)) {
      if (current === target) return true;
    }
    return false;
  };
  const enabledOf = (el) => {
    if (el.matches?.(':disabled') || el.getAttribute?.('aria-disabled') === 'true') return false;
    for (let ancestor = el; ancestor; ancestor = composedParent(ancestor)) {
      if (ancestor.hasAttribute?.('inert')) return false;
    }
    return true;
  };
  const checkedOf = (el) => {
    if (el.tagName === 'INPUT' && ['checkbox', 'radio'].includes(el.type)) return el.indeterminate ? 'mixed' : Boolean(el.checked);
    if (['checkbox', 'radio', 'switch'].includes(roleOf(el))) {
      const checked = el.getAttribute('aria-checked');
      if (checked === 'mixed') return 'mixed';
      if (checked === 'true' || checked === 'false') return checked === 'true';
    }
    return null;
  };
  const checkedStateMatches = (el, input) => {
    if (typeof input.checked !== 'boolean') throw new Error('set_checked requires a boolean checked state');
    if (!el || !enabledOf(el)) throw new Error('set_checked requires an enabled target');
    const state = checkedOf(el);
    if (state === null) throw new Error('set_checked requires a checkbox, radio or switch with an observable checked state');
    if (roleOf(el) === 'radio' && !input.checked) throw new Error('Select a different radio option instead of clearing a radio');
    return state === input.checked;
  };
  const requestedSelectOptions = (el, input) => {
    if (!el || el.tagName !== 'SELECT' || !enabledOf(el)) throw new Error('select requires an enabled select element');
    if (input.values != null && input.value != null) throw new Error('Use value or values, not both');
    const values = input.values != null ? input.values : typeof input.value === 'string' ? [input.value] : null;
    if (!Array.isArray(values) || values.length > 100 || values.some(value => typeof value !== 'string' || value.length > 512) || new Set(values).size !== values.length) throw new Error('select requires up to 100 unique option values');
    if (!el.multiple && values.length !== 1) throw new Error('A single-select requires exactly one value');
    return values.map(value => {
      const option = Array.from(el.options).find(option => option.value === value);
      if (!option || option.matches(':disabled')) throw new Error('Requested select option is missing or disabled');
      return option;
    });
  };
  const cssPath = (el) => {
    const parts = [];
    let current = el;
    while (current?.nodeType === 1 && parts.length < 6) {
      let part = current.tagName.toLowerCase();
      if (current.id) { part += `#${CSS.escape(current.id)}`; parts.unshift(part); break; }
      const parent = current.parentElement;
      if (parent) {
        const siblings = Array.from(parent.children).filter((item) => item.tagName === current.tagName);
        if (siblings.length > 1) part += `:nth-of-type(${siblings.indexOf(current) + 1})`;
      }
      parts.unshift(part);
      current = parent;
    }
    return parts.join(' > ');
  };
  const roots = () => {
    const result = [document];
    const visit = (root) => {
      for (const element of root.querySelectorAll?.('*') || []) {
        if (element.shadowRoot) { result.push(element.shadowRoot); visit(element.shadowRoot); }
        if (element.tagName === 'IFRAME') {
          try { if (element.contentDocument) { result.push(element.contentDocument); visit(element.contentDocument); } } catch (_) {}
        }
      }
    };
    visit(document);
    return result;
  };
  const isObservable = (element) => {
    if (String(element.tagName || '').toUpperCase() === 'INPUT' && String(element.type || '').toLowerCase() === 'hidden') return false;
    const rect = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    return rect.width > 0 && rect.height > 0 && style.display !== 'none' && style.visibility !== 'hidden';
  };
  const interactiveElements = (rootList = roots()) => {
    const selector = 'a[href],button,input:not([type="hidden" i]),textarea,select,[contenteditable="true"],[role="button"],[role="link"],[role="textbox"],[role="checkbox"],[role="radio"],[role="switch"],[role="combobox"],[tabindex]';
    const seen = new Set();
    const elements = [];
    for (const root of rootList) {
      for (const element of root.querySelectorAll?.(selector) || []) {
        if (!seen.has(element) && (isObservable(element) || (element.tagName === 'INPUT' && element.type === 'file'))) { seen.add(element); elements.push(element); }
      }
    }
    return elements;
  };
  const dragDestinationElements = (rootList = roots()) => {
    const selector = '[draggable="true"],[ondrop],[ondragover],[data-dropzone],[data-drop-zone],[class*="drop" i],[id*="drop" i]';
    const elements = [];
    for (const root of rootList) {
      for (const element of root.querySelectorAll?.(selector) || []) {
        if (isObservable(element)) elements.push(element);
      }
    }
    return elements;
  };
  const matchingElements = (rootList = roots()) => {
    const seen = new Set();
    const query = runtime.observationOptions.query.toLowerCase();
    return [...interactiveElements(rootList), ...dragDestinationElements(rootList)].filter((element) => {
      if (seen.has(element)) return false;
      seen.add(element);
      return !query || `${textOf(element)} ${roleOf(element)} ${element.tagName.toLowerCase()}`.toLowerCase().includes(query);
    }).map((element, index) => {
      const rect = viewportBoundsOf(element);
      const inViewport = rect.width > 0 && rect.height > 0 && rect.x < innerWidth && rect.y < innerHeight && rect.x + rect.width > 0 && rect.y + rect.height > 0;
      return { element, index, inViewport };
    }).sort((a, b) => Number(b.inViewport) - Number(a.inViewport) || a.index - b.index).map(item => item.element);
  };
  const observableElements = () => matchingElements().slice(runtime.observationOptions.offset, runtime.observationOptions.offset + 300);
  const navigationTargetOf = (el) => {
    if (el.href) return el.href;
    const tag = String(el.tagName || '').toUpperCase();
    const type = String(el.type || (tag === 'BUTTON' ? 'submit' : '')).toLowerCase();
    const submitter = (tag === 'BUTTON' && type === 'submit') || (tag === 'INPUT' && (type === 'submit' || type === 'image'));
    const implicitSubmitInput = tag === 'INPUT' && !['button', 'reset', 'file', 'checkbox', 'radio'].includes(type);
    if ((!submitter && !implicitSubmitInput) || !el.form) return null;
    try {
      const ownerDocument = el.ownerDocument || document;
      const fallbackUrl = ownerDocument.location?.href || location.href;
      return new URL(el.getAttribute?.('formaction') || el.form.getAttribute?.('action') || fallbackUrl, ownerDocument.baseURI).href;
    } catch (_) { return null; }
  };
  const viewportBoundsOf = (el) => {
    const rect = el.getBoundingClientRect();
    let x = rect.x;
    let y = rect.y;
    let ownerWindow = el.ownerDocument?.defaultView;
    while (ownerWindow && ownerWindow !== window.top) {
      try {
        const frame = ownerWindow.frameElement;
        if (!frame) break;
        const frameBounds = frame.getBoundingClientRect();
        x += frameBounds.x;
        y += frameBounds.y;
        ownerWindow = frame.ownerDocument?.defaultView;
      } catch (_) { break; }
    }
    return { x, y, width: rect.width, height: rect.height };
  };
  const frameLimitationsOf = (rootList) => {
    const frames = [];
    let unavailableCount = 0;
    for (const root of rootList) {
      for (const frame of root.querySelectorAll?.('iframe') || []) {
        try { if (frame.contentDocument?.documentElement) continue; } catch (_) {}
        unavailableCount += 1;
        if (frames.length >= 32) continue;
        let url = null;
        try {
          const parsed = new URL(frame.getAttribute('src') || '', frame.ownerDocument.baseURI);
          if (['http:', 'https:'].includes(parsed.protocol)) url = `${parsed.origin}${parsed.pathname}`.slice(0, 2048);
        } catch (_) {}
        frames.push({ reason: 'cross_origin_or_sandboxed_frame', url, title: (frame.title || '').slice(0, 240), visible: isObservable(frame), bounds: viewportBoundsOf(frame) });
      }
    }
    return { unavailableCount, detailsOmitted: unavailableCount > frames.length, frames };
  };
  const describe = (el, ref, optionBudget = 0) => {
    const rect = viewportBoundsOf(el);
    const name = textOf(el);
    const navigationTarget = navigationTargetOf(el);
    const value = valueOf(el);
    return {
      ref,
      tag: String(el.tagName || '').toLowerCase(),
      role: roleOf(el),
      name,
      href: navigationTarget,
      inputType: el.type || null,
      enabled: enabledOf(el),
      value: value === null ? null : value.slice(0, 2000),
      valueTruncated: value === null ? null : value.length > 2000,
      checked: checkedOf(el),
      options: el.tagName === 'SELECT' ? Array.prototype.slice.call(el.options, 0, Math.min(100, optionBudget)).map(option => ({ value: option.value.slice(0, 512), label: option.label.slice(0, 240), selected: option.selected, enabled: !option.matches(':disabled') })) : null,
      optionCount: el.tagName === 'SELECT' ? el.options.length : null,
      selectedValues: el.tagName === 'SELECT' ? Array.prototype.slice.call(el.selectedOptions, 0, 101).map(option => option.value.slice(0, 512)) : null,
      files: el.type === 'file' ? Array.from(el.files || []).slice(0, 20).map(file => ({ name: file.name, size: file.size })) : null,
      fileCount: el.type === 'file' ? el.files?.length || 0 : null,
      visible: rect.width > 0 && rect.height > 0 && getComputedStyle(el).visibility !== 'hidden',
      bounds: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
      locatorFingerprint: {
        tag: String(el.tagName || '').toLowerCase(),
        id: el.id || null,
        testId: el.getAttribute?.('data-testid') || null,
        name: el.getAttribute?.('name') || null,
        href: navigationTarget,
        cssPath: cssPath(el),
        textHash: name,
      },
    };
  };
  const hashText = (value) => {
    let hash = 2166136261;
    for (let index = 0; index < value.length; index += 1) {
      hash ^= value.charCodeAt(index);
      hash = Math.imul(hash, 16777619);
    }
    return (hash >>> 0).toString(16).padStart(8, '0');
  };
  const contextControlSelector = 'a[href],button,input,textarea,select,[contenteditable="true"],[role="button"],[role="link"],[role="textbox"],[role="combobox"],[role="checkbox"],[role="switch"]';
  const contextDiscoveryControls = 'button,input,textarea,select,[contenteditable="true"],[role="button"],[role="textbox"],[role="combobox"],[role="checkbox"],[role="switch"]';
  const contextText = (container, cache) => {
    if (cache.has(container)) return cache.get(container);
    const parts = [];
    const walker = container.ownerDocument.createTreeWalker(container, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      const parent = walker.currentNode.parentElement;
      const value = walker.currentNode.textContent.trim();
      if (!value || !parent || parent.closest(`${contextDiscoveryControls},script,style,noscript,template`)) continue;
      if (!isObservable(parent)) continue;
      parts.push(value);
    }
    const value = parts.join(' ').replace(/\s+/g, ' ');
    cache.set(container, value);
    return value;
  };
  const targetContextFingerprint = (element, cache = new Map()) => {
    if (!element) return '';
    const parts = [String(element.tagName), roleOf(element), element.id || '', element.getAttribute?.('name') || '', navigationTargetOf(element) || ''];
    if (!element.isContentEditable && !['INPUT','TEXTAREA','SELECT'].includes(element.tagName)) parts.push(textOf(element));
    let owner = null;
    for (let ancestor = element; ancestor; ancestor = composedParent(ancestor)) {
      if (ancestor.matches('tr,[role="row"],li,[role="listitem"],article,fieldset,[role="group"],[data-row-id],[data-item-id]')) { owner = ancestor; break; }
    }
    let branch = element;
    if (!owner) {
      for (let parent = composedParent(element); parent && !parent.matches('body,html,main,[role="main"]'); parent = composedParent(parent)) {
        // Do not promote an unrelated toolbar to the entire collection.
        if (parent.querySelector('tr,[role="row"],li,[role="listitem"],article,main')) break;
        if (contextText(parent, cache)) { owner = parent; break; }
        branch = parent;
      }
      if (!owner && branch !== element && branch.querySelectorAll(contextControlSelector).length > 1) owner = branch;
    }
    const appendContext = (context) => {
      parts.push(context.tagName, context.id || '', context.getAttribute('role') || '', contextText(context, cache));
      for (const attribute of ['aria-label','data-id','data-key','data-row-id','data-item-id']) parts.push(context.getAttribute(attribute) || '');
      // A row label is often itself a link, button or readonly input. Retain
      // those identities; only the action target's own editable value may
      // change as part of the approved input sequence.
      for (const control of [context, ...context.querySelectorAll(contextControlSelector)]) {
        if (control === element || !control.matches(contextControlSelector) || !isObservable(control)) continue;
        parts.push(control.tagName, control.id || '', roleOf(control), textOf(control), navigationTargetOf(control) || '', control.getAttribute('name') || '');
        if ('value' in control) parts.push(String(control.value));
        if ('checked' in control) parts.push(String(control.checked));
      }
    };
    if (owner) appendContext(owner);
    else {
      // Bare inline controls can be associated with a sibling label without
      // including a document-wide clock above or below their row.
      const bounds = element.getBoundingClientRect();
      for (const sibling of [branch.previousElementSibling, branch.nextElementSibling]) {
        if (!sibling) continue;
        const rect = sibling.getBoundingClientRect();
        if (rect.bottom > bounds.top && rect.top < bounds.bottom && rect.height <= Math.max(80, bounds.height * 3)) appendContext(sibling);
      }
    }
    for (const attribute of ['aria-labelledby','aria-describedby']) {
      for (const id of (element.getAttribute(attribute) || '').split(/\s+/).filter(Boolean)) {
        const label = element.getRootNode().getElementById?.(id) || element.ownerDocument.getElementById(id);
        parts.push(id, label?.innerText || label?.textContent || '');
      }
    }
    return hashText(parts.join('\u001f'));
  };
  const interactionFingerprintOf = (elements = observableElements()) => {
    const contextCache = new Map();
    const interactiveState = elements.map((element) => {
      const rect = viewportBoundsOf(element);
      const style = getComputedStyle(element);
      return [
        String(element.tagName || '').toLowerCase(),
        element.id || '',
        element.getAttribute?.('name') || '',
        element.getAttribute?.('role') || '',
        element.getAttribute?.('aria-label') || '',
        navigationTargetOf(element) || '',
        element.type || '',
        element.disabled ? 'disabled' : 'enabled',
        String(checkedOf(element)),
        element.type === 'file' ? `${element.files?.length || 0}:${Array.from(element.files || []).slice(0, 20).map(file => `${file.name}:${file.size}`).join('|')}` : '',
        enabledOf(element) ? 'enabled' : 'disabled',
        Number.isInteger(element.selectedIndex) ? String(element.selectedIndex) : '',
        element.tagName === 'SELECT' ? hashText([element.options.length, element.selectedOptions.length, ...Array.prototype.slice.call(element.options, 0, 100).map(option => [option.value, option.selected, option.matches(':disabled')].join(':')), ...Array.prototype.slice.call(element.selectedOptions, 0, 101).map(option => option.value)].join('|')) : '',
        style.display,
        style.visibility,
        style.opacity,
        Math.round(rect.x),
        Math.round(rect.y),
        Math.round(rect.width),
        Math.round(rect.height),
        textOf(element),
        hashText(valueOf(element) || ''),
        targetContextFingerprint(element, contextCache),
      ].join('\u001f');
    }).join('\u001e');
    return `v4|${location.href}|${scrollX}|${scrollY}|${innerWidth}|${innerHeight}|${hashText(interactiveState)}`;
  };
  const domFingerprintOf = (interactionFingerprint = interactionFingerprintOf()) => {
    const bodyText = document.body?.innerText.slice(0, 30000) || '';
    return `${interactionFingerprint}|${hashText(bodyText)}`;
  };
  const actionVerificationBaseline = () => ({
    url: location.href,
    userEpoch: runtime.userEpoch,
    domFingerprint: domFingerprintOf(),
    observationOptions: { ...runtime.observationOptions },
  });
  const coordinateFingerprintOf = (rootList = roots()) => {
    const state = [];
    for (const root of rootList) for (const element of root.querySelectorAll('*')) {
      let feedback = false;
      for (let ancestor = element; ancestor; ancestor = composedParent(ancestor)) {
        if (feedbackNodes.has(ancestor)) { feedback = true; break; }
      }
      if (feedback) continue;
      const rect = viewportBoundsOf(element);
      if (!rect.width || !rect.height || rect.x >= innerWidth || rect.y >= innerHeight
        || rect.x + rect.width <= 0 || rect.y + rect.height <= 0) continue;
      const style = getComputedStyle(element);
      if (style.visibility === 'hidden' || style.display === 'none' || style.pointerEvents === 'none') continue;
      let id = runtime.geometryIds.get(element);
      if (!id) { id = ++runtime.nextGeometryId; runtime.geometryIds.set(element, id); }
      state.push([id, rect.x, rect.y, rect.width, rect.height, style.display, style.visibility,
        style.pointerEvents, style.position, style.zIndex, style.transform, style.clipPath,
        style.borderRadius, style.overflowX, style.overflowY, element.scrollLeft, element.scrollTop].join('|'));
    }
    return hashText(state.join('\n'));
  };
  runtime.observe = (options = {}) => {
    const query = typeof options.query === 'string' ? options.query.trim() : '';
    const offset = options.offset ?? 0;
    if ([...query].length > 240 || !Number.isSafeInteger(offset) || offset < 0) throw new Error('Invalid browser observation query or offset');
    runtime.observationOptions = { query, offset };
    runtime.refs = new Map();
    let optionBudget = 400;
    const rootList = roots();
    const coordinateFingerprint = coordinateFingerprintOf(rootList);
    const matches = matchingElements(rootList);
    const selected = matches.slice(offset, offset + 300);
    const elements = selected.map((el) => {
      // Screenshot confirmation and settling may take further snapshots. A
      // reference belongs to the actual element, not the snapshot counter.
      let ref = runtime.refIds.get(el);
      if (!ref) {
        ref = `e_${++runtime.nextRefId}`;
        runtime.refIds.set(el, ref);
      }
      runtime.refs.set(ref, el);
      const description = describe(el, ref, optionBudget);
      optionBudget -= description.options?.length || 0;
      return description;
    });
    // This synchronous capture has one selection and one interaction digest.
    // Native screenshot confirmation still performs a separate fresh capture.
    const interactionFingerprint = interactionFingerprintOf(selected);
    return {
      url: location.href,
      title: document.title,
      readyState: document.readyState,
      text: document.body ? document.body.innerText.slice(0, 30000) : '',
      viewport: { width: innerWidth, height: innerHeight, deviceScaleFactor: devicePixelRatio },
      historyLength: history.length,
      userEpoch: runtime.userEpoch,
      domFingerprint: domFingerprintOf(interactionFingerprint),
      interactionFingerprint,
      coordinateFingerprint,
      elements,
      observationCoverage: { query, offset, returned: elements.length, totalMatches: matches.length, hasMore: offset + elements.length < matches.length, nextOffset: offset + elements.length < matches.length ? offset + elements.length : null },
      frameLimitations: frameLimitationsOf(rootList),
    };
  };
  const validateAction = (input) => {
    if (input.userEpoch !== runtime.userEpoch) throw new Error('stale observation: user interacted with the page');
    // Passive clocks and FPS counters do not revoke unchanged targets.
    // Actionable state, exact element identity and user takeover still do.
    if (input.interactionFingerprint !== interactionFingerprintOf()) throw new Error('stale observation: interactive page state changed');
    const el = input.targetRef ? runtime.refs.get(input.targetRef) : null;
    const end = input.endRef ? runtime.refs.get(input.endRef) : null;
    if (input.targetRef && (!el || !el.isConnected)) throw new Error('stale observation: target disappeared');
    if (input.endRef && (!end || !end.isConnected)) throw new Error('stale observation: drag destination disappeared');
    const verify = (element, ref, expected, label) => {
      if (!element || !expected) return;
      const current = describe(element, ref);
      if (!current.enabled) throw new Error('Browser target is disabled');
      if (!current.visible && input.action !== 'upload_files') throw new Error('Browser target is not visible');
      if (current.role !== expected.role || current.name !== expected.name) {
        throw new Error('stale observation: target identity changed');
      }
      const before = expected.bounds;
      const after = current.bounds;
      if (['x','y','width','height'].some((key) => Math.abs(Number(before[key]) - Number(after[key])) > 4)) {
        throw new Error(`stale observation: ${label} bounds changed`);
      }
    };
    verify(el, input.targetRef, input.expected, 'target');
    verify(end, input.endRef, input.expectedEnd, 'drag destination');
    if (input.action === 'select') requestedSelectOptions(el, input);
    if (input.action === 'set_checked') checkedStateMatches(el, input);
    if (input.action === 'upload_files') {
      if (!el || el.tagName !== 'INPUT' || el.type !== 'file' || !enabledOf(el)) throw new Error('upload_files requires an enabled file input');
      if (!Array.isArray(input.files) || input.files.length > 20 || (!el.multiple && input.files.length > 1)) throw new Error('The file input does not accept this file count');
    }
    return { el, end };
  };
  const centerOf = (el) => {
    const rect = el.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  };
  // Feedback is presentation-only. Its completion cannot authorize input.
  const feedbackNodes = new Set();
  const captureTokens = new Map();
  let presentation = null;
  const trackFeedback = (node) => {
    feedbackNodes.add(node);
    if (captureTokens.size) node.style.setProperty('visibility', 'hidden', 'important');
    return node;
  };
  const cancelPresentation = (id) => {
    if (!presentation || presentation.id !== id) return false;
    const pending = presentation.status === 'pending';
    presentation.status = 'cancelled';
    runtime.agentCursor?.getAnimations?.().forEach(animation => animation.cancel());
    if (pending) {
      runtime.agentCursor?.remove(); runtime.agentCursor = null;
      runtime.cursorDocument = null; runtime.cursorPoint = null;
    }
    return true;
  };
  runtime.presentationStatus = (id) => ({ id, status: presentation?.id === id ? presentation.status : 'unavailable' });
  runtime.cancelPresentation = cancelPresentation;
  runtime.beginCapture = (id) => {
    captureTokens.set(id, false);
    for (const node of feedbackNodes) {
      if (!node.isConnected) feedbackNodes.delete(node);
      else node.style.setProperty('visibility', 'hidden', 'important');
    }
    if (!feedbackNodes.size) { captureTokens.set(id, true); return { id }; }
    // Wait for the hidden state to reach a rendered frame before native capture.
    requestAnimationFrame(() => requestAnimationFrame(() => { if (captureTokens.has(id)) captureTokens.set(id, true); }));
    return { id };
  };
  runtime.captureStatus = (id) => ({ id, status: captureTokens.get(id) ? 'ready' : 'pending' });
  runtime.endCapture = (id) => {
    captureTokens.delete(id);
    if (!captureTokens.size) for (const node of feedbackNodes) {
      if (!node.isConnected) feedbackNodes.delete(node);
      else node.style.removeProperty('visibility');
    }
    return true;
  };
  const ensureAgentCursor = (ownerDocument) => {
    if (runtime.agentCursor?.isConnected && runtime.cursorDocument === ownerDocument) return runtime.agentCursor;
    runtime.agentCursor?.remove();
    const cursor = ownerDocument.createElement('div');
    cursor.setAttribute('data-nexa-agent-cursor', 'true');
    cursor.setAttribute('aria-hidden', 'true');
    cursor.style.cssText = `position:fixed;left:${-pointerAppearance.hotspot}px;top:${-pointerAppearance.hotspot}px;width:${pointerAppearance.size}px;height:${pointerAppearance.size}px;z-index:2147483647;pointer-events:none;will-change:transform;contain:layout paint style;`;
    cursor.innerHTML = pointerAppearance.svg;
    (ownerDocument.documentElement || ownerDocument.body).appendChild(cursor);
    trackFeedback(cursor);
    runtime.agentCursor = cursor;
    runtime.cursorDocument = ownerDocument;
    runtime.cursorPoint = null;
    return cursor;
  };
  runtime.setPointerAppearance = (appearance) => {
    pointerAppearance = appearance;
    if (runtime.agentCursor?.isConnected) runtime.agentCursor.innerHTML = appearance.svg;
  };
  const moveAgentCursor = (el, via = null, currentPresentation = null, destinationPoint = null, originPoint = null) => {
    const finish = (status) => {
      if (presentation === currentPresentation && currentPresentation?.status === 'pending') currentPresentation.status = status;
    };
    if (!el) { finish('arrived'); return 0; }
    const ownerDocument = el.ownerDocument || document;
    const ownerWindow = ownerDocument.defaultView || window;
    const cursor = ensureAgentCursor(ownerDocument);
    const to = destinationPoint || centerOf(el);
    const viaPoint = via ? originPoint || centerOf(via) : null;
    const from = runtime.cursorPoint || {
      x: Math.max(18, Number(ownerWindow.innerWidth || 0) * 0.22),
      y: Math.max(22, Number(ownerWindow.innerHeight || 0) * 0.18),
    };
    const distance = viaPoint
      ? Math.hypot(viaPoint.x - from.x, viaPoint.y - from.y) + Math.hypot(to.x - viaPoint.x, to.y - viaPoint.y)
      : Math.hypot(to.x - from.x, to.y - from.y);
    const reduced = pointerAppearance.reducedMotion || ownerWindow.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    const duration = reduced ? 0 : Math.round(Math.min(520, Math.max(180, 150 + distance * 0.36)));
    const translate = (point) => `translate3d(${point.x}px,${point.y}px,0)`;
    cursor.getAnimations?.().forEach((animation) => animation.cancel());
    if (duration > 0 && cursor.animate) {
      const bend = Math.min(70, Math.max(18, distance * 0.14));
      const frames = viaPoint ? [
        { transform: translate(from), offset: 0 },
        { transform: translate(viaPoint), offset: 0.48 },
        { transform: translate(to), offset: 1 },
      ] : [
        { transform: translate(from) },
        { transform: translate({ x: from.x + (to.x - from.x) * 0.55, y: from.y + (to.y - from.y) * 0.45 - bend }) },
        { transform: translate(to) },
      ];
      const motion = cursor.animate(frames, { duration, easing: 'cubic-bezier(.22,.8,.24,1)', fill: 'forwards' });
      motion.finished.then(() => finish(cursor.isConnected ? 'arrived' : 'unavailable'), () => finish('cancelled'));
    } else {
      finish('arrived');
    }
    cursor.style.transform = translate(to);
    runtime.cursorPoint = to;
    return duration;
  };
  const mouseButtonOf = (input) => ({ left: 0, middle: 1, right: 2 })[input.button] ?? 0;
  const pressedButtonsOf = (input) => ({ left: 1, middle: 4, right: 2 })[input.button] ?? 1;
  const pointerOptions = (input, point, detail = 1, pressed = false) => ({
    bubbles: true,
    cancelable: true,
    composed: true,
    clientX: point.x,
    clientY: point.y,
    button: mouseButtonOf(input),
    buttons: pressed ? pressedButtonsOf(input) : 0,
    pointerId: 1,
    pointerType: 'mouse',
    isPrimary: true,
    detail,
    altKey: input.modifiers?.includes('Alt') || false,
    ctrlKey: input.modifiers?.includes('Control') || false,
    metaKey: input.modifiers?.includes('Meta') || false,
    shiftKey: input.modifiers?.includes('Shift') || false,
  });
  const dispatchPointer = (el, type, input, point, detail = 1, pressed = false) => {
    const realm = el.ownerDocument?.defaultView || window;
    const EventType = type.startsWith('pointer') && realm.PointerEvent ? realm.PointerEvent : realm.MouseEvent;
    el.dispatchEvent(new EventType(type, pointerOptions(input, point, detail, pressed)));
  };
  const pulseAt = (el, point) => {
    const ownerDocument = el.ownerDocument || document;
    const pulse = ownerDocument.createElement('div');
    pulse.setAttribute('data-nexa-agent-click', 'true');
    pulse.style.cssText = `position:fixed;z-index:2147483646;pointer-events:none;left:${point.x - 26}px;top:${point.y - 26}px;width:52px;height:52px;border:2.5px solid ${pointerAppearance.accent};border-radius:999px;box-sizing:border-box;`;
    (ownerDocument.documentElement || ownerDocument.body).appendChild(pulse);
    trackFeedback(pulse);
    const reduced = pointerAppearance.reducedMotion || (ownerDocument.defaultView || window).matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    const animation = !reduced && pulse.animate?.([
      { opacity: .88, transform: 'scale(.1538)' },
      { opacity: 0, transform: 'scale(1)' },
    ], { duration: 260, easing: 'ease-out' });
    if (animation) animation.onfinish = () => pulse.remove(); else setTimeout(() => pulse.remove(), 260);
  };
  const hoverAt = (el, input, point) => {
    for (const type of ['pointerover', 'mouseover', 'pointermove', 'mousemove']) dispatchPointer(el, type, input, point);
  };
  const clickAt = (el, input, detail = 1) => {
    const point = centerOf(el);
    hoverAt(el, input, point);
    dispatchPointer(el, 'pointerdown', input, point, detail, true);
    dispatchPointer(el, 'mousedown', input, point, detail, true);
    dispatchPointer(el, 'pointerup', input, point, detail);
    dispatchPointer(el, 'mouseup', input, point, detail);
    const button = mouseButtonOf(input);
    if (button === 0 && !(input.modifiers?.length)) el.click();
    else dispatchPointer(el, button === 1 ? 'auxclick' : button === 2 ? 'contextmenu' : 'click', input, point, detail);
    pulseAt(el, point);
  };
  const dragBetween = (source, destination, input) => {
    const realm = source.ownerDocument?.defaultView || window;
    const from = centerOf(source);
    const to = centerOf(destination);
    let transfer = null;
    try { transfer = realm.DataTransfer ? new realm.DataTransfer() : null; } catch (_) {}
    const dragEvent = (target, type, point) => {
      if (realm.DragEvent) target.dispatchEvent(new realm.DragEvent(type, { ...pointerOptions(input, point, 1, true), dataTransfer: transfer }));
      else dispatchPointer(target, type, input, point, 1, true);
    };
    hoverAt(source, input, from);
    dispatchPointer(source, 'pointerdown', input, from, 1, true);
    dispatchPointer(source, 'mousedown', input, from, 1, true);
    dragEvent(source, 'dragstart', from);
    const ownerDocument = source.ownerDocument || document;
    for (let step = 1; step <= 8; step += 1) {
      const progress = step / 8;
      const point = { x: from.x + (to.x - from.x) * progress, y: from.y + (to.y - from.y) * progress };
      const target = ownerDocument === destination.ownerDocument
        ? deepElementFromPoint(ownerDocument, point.x, point.y) || destination
        : destination;
      dispatchPointer(target, 'pointermove', input, point, 1, true);
      dispatchPointer(target, 'mousemove', input, point, 1, true);
    }
    dragEvent(destination, 'dragenter', to);
    dragEvent(destination, 'dragover', to);
    dragEvent(destination, 'drop', to);
    dragEvent(source, 'dragend', to);
    dispatchPointer(destination, 'pointerup', input, to);
    dispatchPointer(destination, 'mouseup', input, to);
    pulseAt(destination, to);
  };
  runtime.invalidateForUserTakeover = () => {
    if (presentation) cancelPresentation(presentation.id);
    runtime.userEpoch += 1;
    runtime.refs = new Map();
    runtime.agentCursor?.remove();
    runtime.agentCursor = null;
    runtime.cursorDocument = null;
    runtime.cursorPoint = null;
  };
  runtime.previewAction = (input) => {
    const { el, end } = validateAction(input);
    if (input.pointerAppearance) runtime.setPointerAppearance(input.pointerAppearance);
    const destination = input.action === 'drag' ? end : el;
    if (presentation) cancelPresentation(presentation.id);
    const current = { id: String(input.presentationId || ''), status: 'pending' };
    presentation = current;
    return { durationMs: moveAgentCursor(destination, input.action === 'drag' ? el : null, current, input.action === 'drag' ? input.endPoint : input.point, input.point), presentationId: current.id, action: input.action };
  };
  runtime.validateAction = (input) => { validateAction(input); return true; };
  const viewportPoint = (point) => {
    if (!point || !Number.isFinite(point.x) || !Number.isFinite(point.y)
      || point.x < 0 || point.y < 0 || point.x >= innerWidth || point.y >= innerHeight) {
      throw new Error('Browser coordinates must be finite CSS pixels inside the observed viewport');
    }
    return { x: point.x, y: point.y };
  };
  runtime.resolvePointerTarget = (input, coordinates) => {
    validateAction(input);
    if (input.coordinateFingerprint !== coordinateFingerprintOf()) {
      throw new Error('stale observation: screenshot coordinate geometry changed');
    }
    const point = viewportPoint(coordinates);
    let target = deepElementFromPoint(document, point.x, point.y);
    if (!target || target.tagName === 'IFRAME') throw new Error('Observe and use an element ref for a frame target');
    // A screenshot may point at text inside a link or button. Bind the action
    // to that control so native navigation authorization sees its href/form.
    for (let ancestor = target; ancestor; ancestor = composedParent(ancestor)) {
      if (ancestor.matches('a[href],button,input,textarea,select,[role="button"],[role="link"]')) {
        target = ancestor;
        break;
      }
    }
    if (!enabledOf(target)) throw new Error('Browser target is disabled');
    let ref = runtime.refIds.get(target);
    if (!ref) { ref = `e_${++runtime.nextRefId}`; runtime.refIds.set(target, ref); }
    runtime.refs.set(ref, target);
    return describe(target, ref);
  };
  runtime.prepareNativePointer = (input) => {
    const { el } = validateAction(input);
    if (!el) throw new Error('Browser pointer action requires a target');
    if (input.action === 'set_checked' && checkedStateMatches(el, input)) {
      return { stateMatched: true, verificationBaseline: actionVerificationBaseline() };
    }
    const targetContext = targetContextFingerprint(el);
    const ownerDocument = el.ownerDocument || document;
    const ownerWindow = ownerDocument.defaultView || window;
    let rect = el.getBoundingClientRect();
    if (!input.point && (rect.top < 0 || rect.left < 0 || rect.bottom > ownerWindow.innerHeight || rect.right > ownerWindow.innerWidth)) {
      el.scrollIntoView({ behavior: 'instant', block: 'center', inline: 'center' });
      rect = el.getBoundingClientRect();
    }
    const viewportBounds = viewportBoundsOf(el);
    const requestedPoint = input.point ? viewportPoint(input.point) : null;
    const point = requestedPoint ? { x: requestedPoint.x - viewportBounds.x + rect.x, y: requestedPoint.y - viewportBounds.y + rect.y }
      : { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
    if (point.x < 0 || point.y < 0 || point.x >= ownerWindow.innerWidth || point.y >= ownerWindow.innerHeight) {
      throw new Error('Browser pointer target is outside the viewport');
    }
    const hit = deepElementFromPoint(ownerDocument, point.x, point.y);
    if (!hit || !composedContains(el, hit)) {
      throw new Error('Browser pointer target is covered by another element');
    }
    if (targetContext !== targetContextFingerprint(el)) throw new Error('stale observation: target context changed during preparation');
    return {
      bounds: requestedPoint ? { ...requestedPoint, width: 0, height: 0 } : viewportBoundsOf(el),
      targetContext,
      targetRef: input.targetRef,
      verificationBaseline: actionVerificationBaseline(),
    };
  };
  runtime.prepareTrustedDrag = (input) => {
    const { el, end } = validateAction(input);
    if (!el || !end) throw new Error('Browser drag requires both source and destination');
    if (el.ownerDocument !== end.ownerDocument) throw new Error('Browser drag endpoints must belong to the same document');
    // Both points must remain visible together; scrolling one endpoint into
    // view can invalidate the other. Never silently drag to shifted pixels.
    const viewportCenter = (element) => { const bounds = viewportBoundsOf(element); return { x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2 }; };
    const start = runtime.prepareNativePointer({ ...input, point: input.point || viewportCenter(el) });
    const finish = runtime.prepareNativePointer({ ...input, targetRef: input.endRef, expected: input.expectedEnd, point: input.endPoint || viewportCenter(end) });
    return { ...start, endBounds: finish.bounds, endRef: finish.targetRef, endContext: finish.targetContext };
  };
  runtime.prepareTrustedText = (input) => {
    const { el } = validateAction(input);
    if (!el) throw new Error('Trusted browser text input requires a target');
    const targetContext = targetContextFingerprint(el);
    const ownerDocument = el.ownerDocument || document;
    const ownerWindow = ownerDocument.defaultView || window;
    const editable = el.isContentEditable
      || el instanceof ownerWindow.HTMLInputElement
      || el instanceof ownerWindow.HTMLTextAreaElement;
    if (!editable) throw new Error('Trusted browser text input requires an editable target');
    el.focus();
    if (typeof el.select === 'function') {
      el.select();
    } else {
      const range = ownerDocument.createRange();
      range.selectNodeContents(el);
      const selection = ownerWindow.getSelection();
      selection?.removeAllRanges();
      selection?.addRange(range);
    }
    if (targetContext !== targetContextFingerprint(el)) throw new Error('stale observation: target context changed during preparation');
    return {
      focused: deepActiveElement(ownerDocument) === el,
      targetContext,
      targetRef: input.targetRef,
      verificationBaseline: actionVerificationBaseline(),
    };
  };
  runtime.prepareTrustedKey = (input) => {
    const { el } = validateAction(input);
    const ownerDocument = el?.ownerDocument || document;
    const target = el || deepActiveElement(ownerDocument) || ownerDocument.body;
    const targetContext = targetContextFingerprint(target);
    target?.focus?.();
    if (targetContext !== targetContextFingerprint(target)) throw new Error('stale observation: target context changed during preparation');
    return {
      focused: deepActiveElement(ownerDocument) === target,
      targetContext,
      targetRef: input.targetRef || runtime.refIds.get(target),
      verificationBaseline: actionVerificationBaseline(),
    };
  };
  runtime.prepareTrustedUpload = (input) => {
    const { el } = validateAction(input);
    return { targetRef: input.targetRef, targetContext: targetContextFingerprint(el), verificationBaseline: actionVerificationBaseline() };
  };
  runtime.act = (input) => {
    const { el, end } = validateAction(input);
    let inputSent = true;
    runtime.synthetic = true;
    try {
      if (input.action === 'move' || input.action === 'hover') hoverAt(el, input, centerOf(el));
      else if (input.action === 'click') clickAt(el, input);
      else if (input.action === 'set_checked') {
        if (!checkedStateMatches(el, input)) clickAt(el, input);
        else inputSent = false;
      }
      else if (input.action === 'double_click') {
        clickAt(el, input, 1);
        clickAt(el, input, 2);
        dispatchPointer(el, 'dblclick', input, centerOf(el), 2);
      } else if (input.action === 'drag') dragBetween(el, end, input);
      else if (input.action === 'type') {
        el.focus();
        const realm = el.ownerDocument?.defaultView || window;
        if (el.isContentEditable) {
          el.textContent = input.text || '';
        } else {
          const prototype = el instanceof realm.HTMLTextAreaElement ? realm.HTMLTextAreaElement.prototype : realm.HTMLInputElement.prototype;
          const setter = Object.getOwnPropertyDescriptor(prototype, 'value')?.set;
          if (setter) setter.call(el, input.text || ''); else el.value = input.text || '';
        }
        el.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText', data: input.text || '' }));
        el.dispatchEvent(new Event('change', { bubbles: true }));
      } else if (input.action === 'select') {
        const desired = new Set(requestedSelectOptions(el, input));
        const options = Array.from(el.options);
        if (options.some(option => option.selected !== desired.has(option))) {
          for (const option of options) option.selected = desired.has(option);
          const realm = el.ownerDocument.defaultView || window;
          el.dispatchEvent(new realm.Event('input', { bubbles: true }));
          el.dispatchEvent(new realm.Event('change', { bubbles: true }));
        } else inputSent = false;
      } else if (input.action === 'press') {
        const target = el || deepActiveElement(document) || document.body;
        const realm = target.ownerDocument?.defaultView || window;
        const key = String(input.key || '');
        if (key === 'Enter') {
          if (target instanceof realm.HTMLAnchorElement || target instanceof realm.HTMLButtonElement || ['submit', 'image'].includes(String(target.type || '').toLowerCase())) {
            target.click();
          } else if (target instanceof realm.HTMLInputElement && target.form) {
            target.form.requestSubmit();
          } else {
            throw new Error('Unsupported browser key for this element: Enter');
          }
        } else if (key === 'Tab') {
          const focusable = interactiveElements();
          const index = Math.max(-1, focusable.indexOf(target));
          focusable[(index + 1) % focusable.length]?.focus();
        } else if (key === ' ' || key === 'Space' || key === 'Spacebar') {
          const tag = String(target.tagName || '').toUpperCase();
          const type = String(target.type || '').toLowerCase();
          if (['A', 'BUTTON'].includes(tag) || ['checkbox', 'radio'].includes(type)) target.click();
          else throw new Error('Unsupported browser key for this element: Space');
        } else if (key === 'Escape' || key === 'Esc') {
          target.blur?.();
        } else if ((key === 'ArrowDown' || key === 'ArrowUp') && target instanceof realm.HTMLSelectElement) {
          const delta = key === 'ArrowDown' ? 1 : -1;
          target.selectedIndex = Math.max(0, Math.min(target.options.length - 1, target.selectedIndex + delta));
          target.dispatchEvent(new Event('input', { bubbles: true }));
          target.dispatchEvent(new Event('change', { bubbles: true }));
        } else if (['Home', 'End', 'PageUp', 'PageDown'].includes(key)) {
          const amount = key === 'Home' ? -document.documentElement.scrollHeight
            : key === 'End' ? document.documentElement.scrollHeight
            : key === 'PageUp' ? -innerHeight : innerHeight;
          window.scrollBy(0, amount);
        } else {
          throw new Error(`Unsupported browser key: ${key}`);
        }
      } else if (input.action === 'scroll') {
        window.scrollBy(Number(input.scrollX || 0), Number(input.scrollY || 0));
      }
    } finally { runtime.synthetic = false; }
    return { inputSent };
  };
  const clearOverlay = () => {
    runtime.overlay?.remove();
    runtime.overlay = null;
  };
  const showOverlay = (rect, label) => {
    clearOverlay();
    const overlay = document.createElement('div');
    overlay.style.cssText = `position:fixed;z-index:2147483647;pointer-events:none;left:${rect.x}px;top:${rect.y}px;width:${rect.width}px;height:${rect.height}px;border:2px solid #22d3ee;background:rgba(34,211,238,.12);box-shadow:0 0 0 1px rgba(8,47,73,.7)`;
    if (label) overlay.setAttribute('data-nexa-label', label);
    document.documentElement.appendChild(overlay);
    runtime.overlay = overlay;
  };
  const pickMessageMarker = '__NEXA_BROWSER_PICK_BRIDGE__';
  const pickMessageToken = __NEXA_PICK_TOKEN__;
  const apply = Reflect.apply;
  const nativePostMessage = window.postMessage;
  const nativeStopImmediatePropagation = Event.prototype.stopImmediatePropagation;
  const postPickMessage = (target, message) => {
    apply(nativePostMessage, target, [{ marker: pickMessageMarker, token: pickMessageToken, ...message }, '*']);
  };
  const applyPickMode = (mode) => {
    runtime.pickMode = mode;
    runtime.pendingArtifact = null;
    runtime.regionStart = null;
    clearOverlay();
  };
  const childFrames = () => Array.from(document.querySelectorAll('iframe'))
    .map((frame) => ({ frame, target: frame.contentWindow }))
    .filter(({ target }) => Boolean(target));
  const broadcastPick = (message) => {
    for (const { target } of childFrames()) {
      try { postPickMessage(target, message); } catch (_) {}
    }
  };
  const publishArtifact = (artifact) => {
    applyPickMode(null);
    if (window === window.top) {
      runtime.pendingArtifact = artifact;
      broadcastPick({ kind: 'cancel' });
    } else {
      postPickMessage(window.parent, { kind: 'artifact', artifact });
    }
  };
  runtime.beginPick = (mode) => { applyPickMode(mode); broadcastPick({ kind: 'begin', mode }); };
  runtime.cancelPick = () => { applyPickMode(null); broadcastPick({ kind: 'cancel' }); };
  runtime.takeArtifact = () => { const artifact = runtime.pendingArtifact; runtime.pendingArtifact = null; return artifact; };
  runtime.selectedText = () => String(window.getSelection?.() || '').trim().slice(0, 10000);

  addEventListener('message', (event) => {
    const message = event.data;
    if (!message || message.marker !== pickMessageMarker || message.token !== pickMessageToken) return;
    apply(nativeStopImmediatePropagation, event, []);
    if (event.source === window.parent && window !== window.top) {
      if (message.kind === 'begin') {
        applyPickMode(message.mode);
        broadcastPick({ kind: 'begin', mode: message.mode });
      } else if (message.kind === 'cancel') {
        applyPickMode(null);
        broadcastPick({ kind: 'cancel' });
      }
      return;
    }
    if (message.kind !== 'artifact') return;
    const child = childFrames().find(({ target }) => target === event.source);
    if (!child || !message.artifact) return;
    const artifact = { ...message.artifact };
    if (artifact.bounds) {
      const frameBounds = child.frame.getBoundingClientRect();
      artifact.bounds = {
        ...artifact.bounds,
        x: Number(artifact.bounds.x || 0) + frameBounds.x,
        y: Number(artifact.bounds.y || 0) + frameBounds.y,
      };
    }
    if (window === window.top) {
      applyPickMode(null);
      runtime.pendingArtifact = artifact;
      broadcastPick({ kind: 'cancel' });
    } else {
      postPickMessage(window.parent, { kind: 'artifact', artifact });
    }
  }, true);
  addEventListener('load', (event) => {
    const frame = event.target;
    if (!runtime.pickMode || String(frame?.tagName || '').toUpperCase() !== 'IFRAME') return;
    try { if (frame.contentWindow) postPickMessage(frame.contentWindow, { kind: 'begin', mode: runtime.pickMode }); } catch (_) {}
  }, true);

  addEventListener('pointermove', (event) => {
    if (runtime.pickMode !== 'element') return;
    const target = event.composedPath?.().find((item) => item instanceof Element);
    if (target) showOverlay(target.getBoundingClientRect(), `${roleOf(target)} ${textOf(target)}`);
  }, true);
  addEventListener('pointerdown', (event) => {
    if (
      !runtime.synthetic
      && event.isTrusted
      && !window.__NEXA_TRUSTED_INPUT_GUARD__?.expects('pointerdown', event)
    ) {
      runtime.userEpoch += 1;
    }
    if (runtime.pickMode === 'region') {
      if (!event.isTrusted) return;
      runtime.regionStart = { x: event.clientX, y: event.clientY };
      event.preventDefault(); event.stopImmediatePropagation();
    }
  }, true);
  addEventListener('pointermove', (event) => {
    if (runtime.pickMode !== 'region' || !runtime.regionStart) return;
    const x = Math.min(runtime.regionStart.x, event.clientX);
    const y = Math.min(runtime.regionStart.y, event.clientY);
    showOverlay({ x, y, width: Math.abs(event.clientX - runtime.regionStart.x), height: Math.abs(event.clientY - runtime.regionStart.y) }, 'Selected region');
    event.preventDefault(); event.stopImmediatePropagation();
  }, true);
  addEventListener('pointerup', (event) => {
    if (runtime.pickMode !== 'region' || !runtime.regionStart) return;
    if (!event.isTrusted) return;
    const bounds = { x: Math.min(runtime.regionStart.x, event.clientX), y: Math.min(runtime.regionStart.y, event.clientY), width: Math.abs(event.clientX - runtime.regionStart.x), height: Math.abs(event.clientY - runtime.regionStart.y) };
    publishArtifact({ kind: 'region', capture: 'coordinatesOnly', url: location.href, title: document.title, bounds, userEpoch: runtime.userEpoch });
    event.preventDefault(); event.stopImmediatePropagation();
  }, true);
  addEventListener('click', (event) => {
    if (runtime.pickMode !== 'element') return;
    if (!event.isTrusted) return;
    const target = event.composedPath?.().find((item) => item instanceof Element);
    if (target) {
      const ref = `picked_${Date.now()}`;
      runtime.refs.set(ref, target);
      publishArtifact({ kind: 'element', url: location.href, title: document.title, userEpoch: runtime.userEpoch, ...describe(target, ref) });
    }
    if (!target) runtime.cancelPick();
    event.preventDefault(); event.stopImmediatePropagation();
  }, true);
  addEventListener('keydown', (event) => {
    if (
      !runtime.synthetic
      && event.isTrusted
      && !window.__NEXA_TRUSTED_INPUT_GUARD__?.expects('keydown', event)
    ) {
      runtime.userEpoch += 1;
    }
    if (event.key === 'Escape' && runtime.pickMode) { runtime.cancelPick(); event.preventDefault(); event.stopImmediatePropagation(); }
  }, true);
  addEventListener('input', (event) => {
    if (
      !runtime.synthetic
      && event.isTrusted
      && !window.__NEXA_TRUSTED_INPUT_GUARD__?.expects('input', event)
    ) {
      runtime.userEpoch += 1;
    }
  }, true);
  addEventListener('wheel', (event) => {
    if (!runtime.synthetic && event.isTrusted && !window.__NEXA_TRUSTED_INPUT_GUARD__?.expects('wheel', event)) {
      runtime.userEpoch += 1;
    }
  }, true);
  const bridge = Object.freeze({
    observe: (options) => runtime.observe(options),
    targetContextFingerprint: (element) => targetContextFingerprint(element),
    resolveTargetRef: (ref) => runtime.refs.get(ref) || null,
    setPointerAppearance: (appearance) => runtime.setPointerAppearance(appearance),
    previewAction: (input) => runtime.previewAction(input),
    presentationStatus: (id) => runtime.presentationStatus(id),
    cancelPresentation: (id) => runtime.cancelPresentation(id),
    beginCapture: (id) => runtime.beginCapture(id),
    captureStatus: (id) => runtime.captureStatus(id),
    endCapture: (id) => runtime.endCapture(id),
    validateAction: (input) => runtime.validateAction(input),
    resolvePointerTarget: (input, point) => runtime.resolvePointerTarget(input, point),
    prepareTrustedDrag: (input) => runtime.prepareTrustedDrag(input),
    prepareNativePointer: (input) => runtime.prepareNativePointer(input),
    prepareTrustedText: (input) => runtime.prepareTrustedText(input),
    prepareTrustedKey: (input) => runtime.prepareTrustedKey(input),
    prepareTrustedUpload: (input) => runtime.prepareTrustedUpload(input),
    act: (input) => runtime.act(input),
    invalidateForUserTakeover: () => runtime.invalidateForUserTakeover(),
    beginPick: (mode) => runtime.beginPick(mode),
    cancelPick: () => runtime.cancelPick(),
    takeArtifact: () => runtime.takeArtifact(),
    selectedText: () => runtime.selectedText(),
  });
  Object.defineProperty(window, '__NEXA_BROWSER_RUNTIME__', {
    value: bridge,
    configurable: false,
    enumerable: false,
    writable: false,
  });
})();
