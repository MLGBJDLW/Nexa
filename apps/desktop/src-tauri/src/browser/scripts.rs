pub fn browser_takeover_script(token: &str) -> String {
    let takeover_url = format!("nexa-user-input://{token}");
    let encoded_url = serde_json::to_string(&takeover_url)
        .expect("browser takeover URL must be JSON serializable");
    let encoded_token =
        serde_json::to_string(token).expect("browser takeover token must be JSON serializable");
    r#"
(() => {
  const marker = '__NEXA_AUTHENTICATED_TAKEOVER__';
  const takeoverUrl = __NEXA_TAKEOVER_URL__;
  const trustedInputMarker = '__NEXA_AUTHENTICATED_TRUSTED_INPUT__';
  const trustedInputToken = __NEXA_TAKEOVER_TOKEN__;
  const apply = Reflect.apply;
  const nativePostMessage = Window.prototype.postMessage;
  const nativeStopImmediatePropagation = Event.prototype.stopImmediatePropagation;
  let pending = false;
  let trustedInputGuard = null;

  const normalizedTrustedBudget = (budget) => {
    if (!budget || typeof budget !== 'object') return null;
    const pointerDown = Number(budget.pointerDown);
    const keyDown = Number(budget.keyDown);
    const input = Number(budget.input);
    const wheel = Number(budget.wheel || 0);
    if (!Number.isInteger(pointerDown) || pointerDown < 0 || pointerDown > 2) return null;
    if (!Number.isInteger(keyDown) || keyDown < 0 || keyDown > 1) return null;
    if (!Number.isInteger(input) || input < 0 || input > 2) return null;
    if (!Number.isInteger(wheel) || wheel < 0 || wheel > 1) return null;
    if (pointerDown + keyDown + input + wheel === 0) return null;
    return { pointerDown, keyDown, input, wheel };
  };

  const normalizedTrustedExpectation = (expected) => {
    if (!expected || typeof expected !== 'object') return null;
    const targetRef = String(expected.targetRef || '');
    const targetContext = String(expected.targetContext || '');
    if (!targetRef || !targetContext) return null;
    if (['pointer', 'drag', 'wheel'].includes(expected.kind)) {
      const x = Number(expected.x);
      const y = Number(expected.y);
      const button = String(expected.button || '');
      if (!Number.isFinite(x) || !Number.isFinite(y) || x < 0 || y < 0) return null;
      if (expected.kind === 'wheel') {
        const deltaX = Number(expected.deltaX), deltaY = Number(expected.deltaY);
        if (![deltaX, deltaY].every(value => Number.isFinite(value) && Math.abs(value) <= 100000)) return null;
        return { kind: 'wheel', x, y, deltaX, deltaY, targetRef, targetContext };
      }
      if (expected.kind === 'drag') {
        const endX = Number(expected.endX), endY = Number(expected.endY);
        const endRef = String(expected.endRef || ''), endContext = String(expected.endContext || '');
        if (![endX, endY].every(value => Number.isFinite(value) && value >= 0) || !endRef || !endContext || button !== 'left') return null;
        return { kind: 'drag', x, y, endX, endY, endRef, endContext, button, targetRef, targetContext };
      }
      if (!['left', 'middle', 'right'].includes(button)) return null;
      return { kind: 'pointer', x, y, button, targetRef, targetContext };
    }
    if (expected.kind === 'key') {
      const key = String(expected.key || '');
      const modifiers = Number(expected.modifiers || 0);
      return key && key.length <= 32 && Number.isInteger(modifiers) && modifiers >= 0 && modifiers <= 15 ? { kind: 'key', key, modifiers, targetRef, targetContext } : null;
    }
    if (expected.kind === 'text') {
      const data = String(expected.data ?? '');
      return data.length <= 262144 ? { kind: 'text', data, targetRef, targetContext } : null;
    }
    if (expected.kind === 'files') {
      const files = expected.files;
      if (!Array.isArray(files) || files.length > 20 || files.some(file => typeof file.name !== 'string' || !file.name || file.name.length > 512 || !Number.isSafeInteger(file.size) || file.size < 0 || file.size > 104857600)) return null;
      return { kind: 'files', files: files.map(file => ({ name: file.name, size: file.size })), targetRef, targetContext };
    }
    return null;
  };

  const observationBridge = () => {
    try { return window.top.__NEXA_BROWSER_RUNTIME__; } catch (_) { return null; }
  };
  const deepElementFromPoint = (root, x, y) => {
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
    for (let current = node; current; current = current.assignedSlot || current.parentElement || current.getRootNode?.().host) {
      if (current === target) return true;
    }
    return false;
  };
  const eventElement = (event) => event.composedPath?.().find(node => node?.nodeType === 1) || event.target;
  const hitMatchesTarget = (target, hit) => {
    if (composedContains(target, hit)) return true;
    for (let ownerWindow = target.ownerDocument.defaultView; ownerWindow && ownerWindow !== window;) {
      try {
        const frame = ownerWindow.frameElement;
        if (frame === hit) return true;
        ownerWindow = frame?.ownerDocument.defaultView;
      } catch (_) { break; }
    }
    return false;
  };
  const guardContextMatches = (finishingDrag = false) => {
    const bridge = observationBridge();
    const guard = trustedInputGuard;
    if (guard?.expected.kind === 'drag' && guard.pointerDown === 0) {
      const end = bridge?.resolveTargetRef(guard.expected.endRef);
      return Boolean(guard.target.isConnected && end?.isConnected
        && bridge.resolveTargetRef(guard.expected.targetRef) === guard.target
        && (!finishingDrag || bridge.targetContextFingerprint(end) === guard.expected.endContext));
    }
    return Boolean(guard && guard.target.isConnected
      && bridge?.resolveTargetRef(guard.expected.targetRef) === guard.target
      && bridge.targetContextFingerprint(guard.target) === guard.expected.targetContext);
  };

  const armTrustedInputLocal = (operationId, budget, expected) => {
    const normalized = normalizedTrustedBudget(budget);
    const normalizedExpected = normalizedTrustedExpectation(expected);
    if (trustedInputGuard && !trustedInputGuard.blocked && trustedInputGuard.expected.kind !== 'drag' && trustedInputGuard.pointerDown + trustedInputGuard.keyDown + trustedInputGuard.input + trustedInputGuard.wheel === 0) trustedInputGuard = null;
    if (!normalized || !normalizedExpected || typeof operationId !== 'string' || !operationId || trustedInputGuard) return false;
    const hit = ['pointer', 'drag', 'wheel'].includes(normalizedExpected.kind)
      ? deepElementFromPoint(document, normalizedExpected.x, normalizedExpected.y)
      : deepActiveElement(document);
    const bridge = observationBridge();
    const target = bridge?.resolveTargetRef(normalizedExpected.targetRef);
    if (!target || (['key', 'text', 'files'].includes(normalizedExpected.kind) && (target === document.body || target === document.documentElement))) return false;
    if (!target.isConnected || bridge.targetContextFingerprint(target) !== normalizedExpected.targetContext) return false;
    const matches = normalizedExpected.kind === 'files'
      ? target.tagName === 'INPUT' && target.type === 'file'
      : hitMatchesTarget(target, hit);
    if (!matches) return false;
    if (normalizedExpected.kind === 'drag') {
      const end = bridge?.resolveTargetRef(normalizedExpected.endRef);
      const endHit = deepElementFromPoint(document, normalizedExpected.endX, normalizedExpected.endY);
      if (!end?.isConnected || bridge.targetContextFingerprint(end) !== normalizedExpected.endContext || !hitMatchesTarget(end, endHit)) return false;
    }
    trustedInputGuard = {
      operationId,
      expected: normalizedExpected,
      target,
      blocked: false,
      pointerCommitted: false,
      ...normalized,
    };
    return true;
  };

  const disarmTrustedInputLocal = (operationId) => {
    if (trustedInputGuard && trustedInputGuard.operationId !== operationId) return false;
    const blocked = trustedInputGuard?.blocked === true;
    trustedInputGuard = null;
    return !blocked;
  };

  const expectationForFrame = (expected, frame) => {
    if (!expected || !['pointer', 'drag', 'wheel'].includes(expected.kind)) return expected;
    const rect = frame.getBoundingClientRect();
    return { ...expected, x: expected.x - rect.left, y: expected.y - rect.top,
      ...(expected.kind === 'drag' ? { endX: expected.endX - rect.left, endY: expected.endY - rect.top } : {}) };
  };

  const relayTrustedInput = (kind, operationId, budget = null, expected = null) => {
    for (const frame of Array.from(document.querySelectorAll('iframe'))) {
      try {
        if (frame.contentWindow) {
          apply(nativePostMessage, frame.contentWindow, [{
            marker: trustedInputMarker,
            token: trustedInputToken,
            kind,
            operationId,
            budget,
            expected: expectationForFrame(expected, frame),
          }, '*']);
        }
      } catch (_) {}
    }
  };

  const trustedInputApi = Object.freeze({
    expects: (type, event) => matchesTrustedInput(type, event),
    arm: (providedToken, operationId, budget, expected) => {
      if (providedToken !== trustedInputToken) return false;
      const armed = armTrustedInputLocal(operationId, budget, expected);
      if (armed) relayTrustedInput('arm', operationId, budget, expected);
      return armed;
    },
    disarm: (providedToken, operationId) => {
      if (providedToken !== trustedInputToken) return false;
      const disarmed = disarmTrustedInputLocal(operationId);
      if (disarmed) relayTrustedInput('disarm', operationId);
      return disarmed;
    },
  });
  Object.defineProperty(window, '__NEXA_TRUSTED_INPUT_GUARD__', {
    value: trustedInputApi,
    configurable: false,
    enumerable: false,
    writable: false,
  });

  addEventListener('message', (event) => {
    const message = event.data;
    if (window === window.top && message?.marker === trustedInputMarker && message.token === trustedInputToken && message.kind === 'blocked') {
      apply(nativeStopImmediatePropagation, event, []);
      if (trustedInputGuard?.operationId === message.operationId) trustedInputGuard.blocked = true;
      return;
    }
    if (window === window.top || event.source !== window.parent) return;
    if (!message || message.marker !== trustedInputMarker || message.token !== trustedInputToken) return;
    apply(nativeStopImmediatePropagation, event, []);
    if (message.kind === 'arm') {
      if (armTrustedInputLocal(message.operationId, message.budget, message.expected)) {
        relayTrustedInput('arm', message.operationId, message.budget, message.expected);
      }
    } else if (message.kind === 'disarm') {
      if (disarmTrustedInputLocal(message.operationId)) {
        relayTrustedInput('disarm', message.operationId);
      }
    }
  }, true);

  const eventTargetsArmedElement = (event) => {
    const target = trustedInputGuard?.target;
    if (!target || target === document.body || target === document.documentElement) return true;
    return composedContains(target, eventElement(event));
  };

  const matchesTrustedInput = (type, event) => {
    if (!trustedInputGuard) return false;
    const budgetKey = ({ pointerdown: 'pointerDown', keydown: 'keyDown', input: 'input', wheel: 'wheel' })[type];
    if (!budgetKey || trustedInputGuard[budgetKey] <= 0) return false;
    const expected = trustedInputGuard.expected;
    if (type === 'pointerdown') {
      const button = ({ 0: 'left', 1: 'middle', 2: 'right' })[event.button];
      return ['pointer', 'drag'].includes(expected.kind)
        && button === expected.button
        && Math.abs(event.clientX - expected.x) <= 2
        && Math.abs(event.clientY - expected.y) <= 2
        && eventTargetsArmedElement(event);
    }
    if (type === 'keydown') {
      const modifiers = (event.altKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) | (event.metaKey ? 4 : 0) | (event.shiftKey ? 8 : 0);
      return expected.kind === 'key' && event.key === expected.key && modifiers === expected.modifiers && eventTargetsArmedElement(event);
    }
    if (type === 'wheel') {
      return expected.kind === 'wheel' && Math.abs(event.clientX - expected.x) <= 2 && Math.abs(event.clientY - expected.y) <= 2
        && Math.abs(event.deltaX - expected.deltaX) <= 1 && Math.abs(event.deltaY - expected.deltaY) <= 1 && eventTargetsArmedElement(event);
    }
    if (expected.kind === 'text') {
      return event.data === expected.data && eventTargetsArmedElement(event);
    }
    if (expected.kind === 'files') {
      const files = eventElement(event)?.files;
      return eventTargetsArmedElement(event) && files?.length === expected.files.length
        && expected.files.every((file, index) => files[index].name === file.name && files[index].size === file.size);
    }
    if (expected.kind === 'key') return eventTargetsArmedElement(event);
    if (expected.kind === 'pointer') {
      const rect = eventElement(event)?.getBoundingClientRect?.();
      return eventTargetsArmedElement(event) && Boolean(rect
        && expected.x >= rect.left && expected.x <= rect.right
        && expected.y >= rect.top && expected.y <= rect.bottom);
    }
    return false;
  };

  const consumeTrustedInput = (type, event) => {
    if (!matchesTrustedInput(type, event)) {
      trustedInputGuard = null;
      return false;
    }
    const budgetKey = ({ pointerdown: 'pointerDown', keydown: 'keyDown', input: 'input', wheel: 'wheel' })[type];
    trustedInputGuard[budgetKey] -= 1;
    return true;
  };

  // Keep the context fence through the actual event, including a row recycle
  // between preparation, pointerdown and click. Unrelated user input still
  // follows the takeover path rather than being swallowed by this allowance.
  for (const type of ['pointerdown','pointermove','pointerup','mousedown','mousemove','mouseup','click','dblclick','dragstart','dragover','drop','keydown','keyup','keypress','beforeinput','wheel']) {
    addEventListener(type, (event) => {
      const guard = trustedInputGuard;
      if (!guard || !event.isTrusted) return;
      if (!guard.blocked && guard.pointerCommitted && type === 'pointerdown' && guard.pointerDown === 0) { trustedInputGuard = null; return; }
      const expected = guard.expected;
      const activationKey = expected.kind === 'key' && [' ', 'Enter'].includes(expected.key);
      const activationControl = guard.target.matches('button,a[href],input[type="button"],input[type="submit"],input[type="checkbox"],input[type="radio"],[role="button"],[role="link"],[role="checkbox"],[role="switch"]');
      const finishingDrag = expected.kind === 'drag' && ['pointerup', 'mouseup', 'drop', 'click'].includes(type);
      const matches = expected.kind === 'drag'
        ? ['pointerdown','pointermove','pointerup','mousedown','mousemove','mouseup','click','dragstart','dragover','drop'].includes(type)
          && event.clientX >= Math.min(expected.x, expected.endX) - 2 && event.clientX <= Math.max(expected.x, expected.endX) + 2
          && event.clientY >= Math.min(expected.y, expected.endY) - 2 && event.clientY <= Math.max(expected.y, expected.endY) + 2
        : expected.kind === 'wheel' ? type === 'wheel' && matchesTrustedInput(type, event)
        : expected.kind === 'pointer'
        ? ['pointerdown','pointerup','mousedown','mouseup','click','dblclick'].includes(type)
          && Math.abs(event.clientX - expected.x) <= 2 && Math.abs(event.clientY - expected.y) <= 2
        : expected.kind === 'text' ? type === 'beforeinput' && event.data === expected.data
        : (type === 'keydown' && event.key === expected.key)
          || (activationKey && !guard.pointerCommitted && activationControl && ['keyup','keypress'].includes(type) && event.key === expected.key)
          || (activationKey && type === 'click' && event.detail === 0)
          || (type === 'beforeinput' && guard.keyDown === 0 && guard.input > 0 && eventTargetsArmedElement(event));
      if (!matches) return;
      if (guard.blocked || !guardContextMatches(finishingDrag)) {
        guard.blocked = true;
        event.preventDefault();
        apply(nativeStopImmediatePropagation, event, []);
        if (window !== window.top) apply(nativePostMessage, window.top, [{ marker: trustedInputMarker, token: trustedInputToken, kind: 'blocked', operationId: guard.operationId }, '*']);
      } else if (type === 'click') guard.pointerCommitted = true;
    }, { capture: true, passive: false });
  }

  const navigateSignal = () => {
    if (pending) return;
    pending = true;
    if (window === window.top) {
      window.location.href = takeoverUrl;
    } else {
      apply(nativePostMessage, window.top, [{ marker, takeoverUrl }, '*']);
    }
    setTimeout(() => { pending = false; }, 100);
  };

  if (window === window.top) {
    addEventListener('message', (event) => {
      if (!event.data || event.data.marker !== marker) return;
      apply(nativeStopImmediatePropagation, event, []);
      if (event.data.takeoverUrl === takeoverUrl) window.location.href = takeoverUrl;
    }, true);
  }

  for (const type of ['pointerdown', 'keydown', 'input', 'wheel', 'touchstart']) {
    addEventListener(type, (event) => {
      if (event.isTrusted && !consumeTrustedInput(type, event)) navigateSignal();
    }, true);
  }
})();
"#
    .replace("__NEXA_TAKEOVER_URL__", &encoded_url)
    .replace("__NEXA_TAKEOVER_TOKEN__", &encoded_token)
}

pub fn browser_init_script(pick_token: &str) -> String {
    let encoded_token =
        serde_json::to_string(pick_token).expect("browser picker token must be JSON serializable");
    BROWSER_INIT_SCRIPT.replace("__NEXA_PICK_TOKEN__", &encoded_token)
}

pub const BROWSER_INIT_SCRIPT: &str = include_str!("interaction_runtime.js");
