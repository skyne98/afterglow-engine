// DOM environment: load LinkeDOM (a spec-compliant pure-JS DOM implementation
// that runs in V8 with no browser) and expose its document/window/etc. globally.
// This REPLACES the hand-rolled browser_shim.js stubs with a real DOM, so the
// Inspector, OrbitControls, and all addons run unmodified.
//
// The one piece LinkeDOM cannot provide is WebGPU on canvas — that's our
// environment (op_create_capture_canvas). We hook canvas.getContext('webgpu')
// to return it. This is an environment piece, not a client-code replacement.

import * as linkedom from './vendor/linkedom/linkedom.mjs';
import {
  CanvasGradient,
  CanvasRenderingContext2D,
  ImageData,
  installCanvas2D,
} from './canvas_2d.ts';

const dom = linkedom.parseHTML(
  globalThis.__documentHTML || '<!DOCTYPE html><html><head></head><body></body></html>',
);
delete globalThis.__documentHTML;
const document = dom.document;
// LinkeDOM may synthesize detached head/body accessors for HTML that places a
// style element between </head> and <body>. Browsers retain the explicit body.
const parsedHead = document.querySelector('head');
const parsedBody = document.querySelector('body');
if (parsedHead && document.head !== parsedHead) {
  Object.defineProperty(document, 'head', { value: parsedHead, configurable: true });
}
if (parsedBody && document.body !== parsedBody) {
  Object.defineProperty(document, 'body', { value: parsedBody, configurable: true });
}
// LinkeDOM can retain an extra synthetic <body> when a style element appears
// between </head> and the explicit body. HTML parsing has one body element;
// merge any duplicate's children into the canonical body and remove it.
for (const body of Array.from(document.querySelectorAll('body'))) {
  if (body === document.body) continue;
  while (body.firstChild) document.body.appendChild(body.firstChild);
  body.remove();
}

// LinkeDOM exposes DOM interfaces as module exports and as non-enumerable
// window properties. Install the complete interface set, not merely the
// enumerable window keys (which omit HTMLElement and most HTML* classes).
for (const [name, value] of Object.entries(linkedom)) {
  if (!(name in globalThis)) globalThis[name] = value;
}
// LinkeDOM dispatch mutates the event phase/current target while traversing its
// own listener tree, so its Event constructors must accompany its EventTarget
// even when the embedding runtime already provides unrelated web events.
globalThis.EventTarget = linkedom.EventTarget;
globalThis.Event = linkedom.Event;
globalThis.CustomEvent = linkedom.CustomEvent;
globalThis.MutationObserver = dom.MutationObserver;
globalThis.addEventListener = dom.addEventListener.bind(dom);
globalThis.removeEventListener = dom.removeEventListener.bind(dom);
globalThis.dispatchEvent = dom.dispatchEvent.bind(dom);
globalThis.window = globalThis;
globalThis.document = document;
globalThis.self = globalThis;
globalThis.CanvasGradient = CanvasGradient;
globalThis.CanvasRenderingContext2D = CanvasRenderingContext2D;
globalThis.ImageData = ImageData;

// LinkeDOM owns JavaScript DOM identity and mutation semantics. Blitz receives
// structured records keyed by out-of-band WeakMap IDs whenever layout is
// forced; no bridge metadata is exposed through attributes or selectors.
const nativeNodeIds = new WeakMap();
const nativeNodes = new Map();
let nextNativeNodeId = 0;
let browserDomEpoch = 1;
let browserDomDirty = true;
let browserFullPaintDirty = true;
let browserDocumentSynced = false;
const browserDirtyNodeIds = new Set();
let scheduleBrowserObservers = () => {};
const ensureNativeNodeId = (node) => {
  let id = nativeNodeIds.get(node);
  if (id === undefined) {
    id = ++nextNativeNodeId;
    nativeNodeIds.set(node, id);
    nativeNodes.set(id, node);
  }
  return id;
};
const observeBrowserDocument = () => browserMutationObserver.observe(document, {
  subtree: true,
  childList: true,
  attributes: true,
  characterData: true,
});
const markBrowserDocumentDirty = (records) => {
  browserDomDirty = true;
  browserDomEpoch++;
  if (records != null && typeof records !== 'string') {
    for (const record of Array.from(records)) {
      const addedNode = Array.from(record.addedNodes ?? []).find((node) => node?.isConnected);
      const target = record.target?.nodeType === 1 ? record.target
        : record.target?.parentElement ?? (addedNode?.nodeType === 1 ? addedNode : addedNode?.parentElement);
      if (!target) { browserFullPaintDirty = true; continue; }
      browserDirtyNodeIds.add(ensureNativeNodeId(target));
      if (target.parentElement) browserDirtyNodeIds.add(ensureNativeNodeId(target.parentElement));
    }
  } else {
    browserFullPaintDirty = true;
  }
  scheduleBrowserObservers();
};
const browserMutationObserver = new MutationObserver(markBrowserDocumentDirty);
observeBrowserDocument();
for (const method of ['insertRule', 'deleteRule', 'replace', 'replaceSync']) {
  const original = globalThis.CSSStyleSheet?.prototype?.[method];
  if (typeof original !== 'function') continue;
  Object.defineProperty(CSSStyleSheet.prototype, method, {
    configurable: true,
    writable: true,
    value(...args) {
      const result = original.apply(this, args);
      markBrowserDocumentDirty();
      return result;
    },
  });
}

const serializeBrowserDocument = () => {
  const pending = browserMutationObserver.takeRecords();
  if (pending.length !== 0) markBrowserDocumentDirty(pending);
  const nodes = [];
  const stylesheetText = (node) => {
    if (node.nodeType !== 1 || node.localName !== 'style') return null;
    try {
      const sheet = node.sheet;
      return sheet ? Array.from(sheet.cssRules).map((rule) => rule.cssText).join('\n') : null;
    } catch {
      return null;
    }
  };
  const visit = (node) => {
    let kind;
    if (node.nodeType === 1) kind = 'element';
    else if (node.nodeType === 3) kind = 'text';
    else if (node.nodeType === 8) kind = 'comment';
    else return;
    const children = Array.from(node.childNodes).filter((child) =>
      child.nodeType === 1 || child.nodeType === 3 || child.nodeType === 8
    );
    nodes.push({
      id: ensureNativeNodeId(node),
      kind,
      localName: kind === 'element' ? node.localName : null,
      namespace: kind === 'element' ? node.namespaceURI : null,
      prefix: kind === 'element' ? node.prefix : null,
      attributes: kind === 'element' ? Array.from(node.attributes).map((attribute) => ({
        localName: attribute.localName || attribute.name,
        namespace: attribute.namespaceURI,
        prefix: attribute.prefix,
        value: attribute.value,
      })) : [],
      text: kind === 'text' ? node.data : null,
      stylesheetText: stylesheetText(node),
      checked: kind === 'element' && node.localName === 'input' && /^(checkbox|radio)$/.test(node.type)
        ? Boolean(node.checked) : null,
      children: children.map(ensureNativeNodeId),
    });
    for (const child of children) visit(child);
  };
  visit(document.documentElement);
  return { nodes };
};

const syncBrowserDocument = (force = false) => {
  const pending = browserMutationObserver.takeRecords();
  if (pending.length !== 0) markBrowserDocumentDirty(pending);
  if (browserDomDirty || force) {
    if (force && !browserDomDirty) browserDomEpoch++;
    const snapshot = serializeBrowserDocument();
    Deno.core.ops.op_sync_browser_document(
      browserDomEpoch,
      snapshot,
      globalThis.__exampleURL,
      Array.from(browserDirtyNodeIds),
      !browserDocumentSynced || (browserFullPaintDirty && browserDirtyNodeIds.size === 0),
    );
    browserDocumentSynced = true;
    browserDomDirty = false;
    browserFullPaintDirty = false;
    browserDirtyNodeIds.clear();
    scheduleBrowserObservers();
  }
};

globalThis.__syncBrowserDocument = syncBrowserDocument;
class DOMRectReadOnly {
  constructor(x = 0, y = 0, width = 0, height = 0) {
    this.x = Number(x); this.y = Number(y);
    this.width = Number(width); this.height = Number(height);
  }
  get top() { return Math.min(this.y, this.y + this.height); }
  get right() { return Math.max(this.x, this.x + this.width); }
  get bottom() { return Math.max(this.y, this.y + this.height); }
  get left() { return Math.min(this.x, this.x + this.width); }
  toJSON() {
    return { x: this.x, y: this.y, width: this.width, height: this.height,
      top: this.top, right: this.right, bottom: this.bottom, left: this.left };
  }
  static from(rect = {}) { return new this(rect.x, rect.y, rect.width, rect.height); }
}
class DOMRect extends DOMRectReadOnly {
  constructor(x = 0, y = 0, width = 0, height = 0) {
    super(x, y, width, height);
  }
}
globalThis.DOMRectReadOnly = DOMRectReadOnly;
globalThis.DOMRect = DOMRect;
Object.defineProperties(document, {
  elementFromPoint: {
    configurable: true,
    value(x, y) {
      syncBrowserDocument();
      const id = Deno.core.ops.op_browser_hit_test(Number(x), Number(y));
      return id == null ? null : nativeNodes.get(id) ?? null;
    },
  },
  elementsFromPoint: {
    configurable: true,
    value(x, y) {
      syncBrowserDocument();
      return Deno.core.ops.op_browser_hit_tests(Number(x), Number(y))
        .map((id) => nativeNodes.get(id))
        .filter((node) => node instanceof Element && node.isConnected);
    },
  },
});
Object.defineProperty(Element.prototype, 'getBoundingClientRect', {
  configurable: true,
  value() {
    if (!this.isConnected) {
      return new DOMRect();
    }
    syncBrowserDocument();
    const rect = Deno.core.ops.op_browser_rect(ensureNativeNodeId(this));
    return new DOMRect(rect.x, rect.y, rect.width, rect.height);
  },
});
const browserTabIndex = (element) => {
  if (element.hasAttribute('tabindex')) {
    const value = Number.parseInt(element.getAttribute('tabindex'), 10);
    return Number.isFinite(value) ? value : 0;
  }
  if (/^(button|input|select|textarea)$/.test(element.localName)) return element.hasAttribute('disabled') ? -1 : 0;
  if ((element.localName === 'a' || element.localName === 'area') && element.hasAttribute('href')) return 0;
  if (element.localName === 'summary' || element.isContentEditable) return 0;
  return -1;
};
const isBrowserFocusable = (element) => {
  if (element.hasAttribute('disabled') && /^(button|input|select|textarea|fieldset)$/.test(element.localName)) return false;
  return element === document.body || browserTabIndex(element) >= 0 || element.hasAttribute('tabindex');
};
HTMLElement.prototype.focus = function focus(_options = undefined) {
  if (!this.isConnected || !isBrowserFocusable(this) || browserActiveElement === this) return;
  syncBrowserDocument();
  const previous = browserActiveElement?.isConnected ? browserActiveElement : null;
  if (Deno.core.ops.op_browser_set_focus(ensureNativeNodeId(this))) scheduleBrowserObservers();
  browserActiveElement = this;
  if (previous) {
    dispatchFocusEvent(previous, 'blur', false, this);
    dispatchFocusEvent(previous, 'focusout', true, this);
  }
  dispatchFocusEvent(this, 'focus', false, previous);
  dispatchFocusEvent(this, 'focusin', true, previous);
};
HTMLElement.prototype.blur = function blur() {
  if (browserActiveElement !== this) return;
  if (Deno.core.ops.op_browser_set_focus(0)) scheduleBrowserObservers();
  browserActiveElement = document.body;
  dispatchFocusEvent(this, 'blur', false, document.body);
  dispatchFocusEvent(this, 'focusout', true, document.body);
};

// LinkeDOM 0.18 exposes HTMLSelectElement.value as getter-only. Browsers also
// provide the setter, which updates option selectedness.
const innerHTML = Object.getOwnPropertyDescriptor(Element.prototype, 'innerHTML');
Object.defineProperty(Element.prototype, 'innerHTML', {
  configurable: true,
  enumerable: innerHTML.enumerable,
  get() { return innerHTML.get.call(this); },
  set(value) { innerHTML.set.call(this, String(value)); },
});

Object.defineProperty(HTMLElement.prototype, 'innerText', {
  configurable: true,
  enumerable: true,
  get() { return this.textContent; },
  set(value) { this.textContent = String(value); },
});

const browserBoxMetrics = (element) => {
  if (!element.isConnected) return null;
  syncBrowserDocument();
  return Deno.core.ops.op_browser_box_metrics(ensureNativeNodeId(element));
};
Object.defineProperties(Element.prototype, {
  clientWidth: { configurable: true, get() { return browserBoxMetrics(this)?.clientWidth ?? 0; } },
  clientHeight: { configurable: true, get() { return browserBoxMetrics(this)?.clientHeight ?? 0; } },
  clientLeft: { configurable: true, get() { return browserBoxMetrics(this)?.clientLeft ?? 0; } },
  clientTop: { configurable: true, get() { return browserBoxMetrics(this)?.clientTop ?? 0; } },
  scrollWidth: { configurable: true, get() { return browserBoxMetrics(this)?.scrollWidth ?? 0; } },
  scrollHeight: { configurable: true, get() { return browserBoxMetrics(this)?.scrollHeight ?? 0; } },
  scrollLeft: {
    configurable: true,
    get() { return browserBoxMetrics(this)?.scrollLeft ?? 0; },
    set(value) {
      if (!this.isConnected) return;
      syncBrowserDocument();
      if (Deno.core.ops.op_browser_set_scroll(ensureNativeNodeId(this), Number(value) || 0, this.scrollTop)) {
        this.dispatchEvent(new Event('scroll'));
        scheduleBrowserObservers();
      }
    },
  },
  scrollTop: {
    configurable: true,
    get() { return browserBoxMetrics(this)?.scrollTop ?? 0; },
    set(value) {
      if (!this.isConnected) return;
      syncBrowserDocument();
      if (Deno.core.ops.op_browser_set_scroll(ensureNativeNodeId(this), this.scrollLeft, Number(value) || 0)) {
        this.dispatchEvent(new Event('scroll'));
        scheduleBrowserObservers();
      }
    },
  },
});
Element.prototype.scrollTo = function scrollTo(leftOrOptions = 0, top = 0) {
  const left = typeof leftOrOptions === 'object' ? leftOrOptions.left ?? this.scrollLeft : leftOrOptions;
  const nextTop = typeof leftOrOptions === 'object' ? leftOrOptions.top ?? this.scrollTop : top;
  if (!this.isConnected) return;
  syncBrowserDocument();
  if (Deno.core.ops.op_browser_set_scroll(ensureNativeNodeId(this), Number(left) || 0, Number(nextTop) || 0)) {
    this.dispatchEvent(new Event('scroll'));
    scheduleBrowserObservers();
  }
};
Element.prototype.scroll = Element.prototype.scrollTo;
Element.prototype.scrollBy = function scrollBy(leftOrOptions = 0, top = 0) {
  const left = typeof leftOrOptions === 'object' ? leftOrOptions.left ?? 0 : leftOrOptions;
  const nextTop = typeof leftOrOptions === 'object' ? leftOrOptions.top ?? 0 : top;
  this.scrollTo(this.scrollLeft + (Number(left) || 0), this.scrollTop + (Number(nextTop) || 0));
};
let browserActiveElement = document.body;
Object.defineProperty(document, 'activeElement', {
  configurable: true,
  get() { return browserActiveElement?.isConnected ? browserActiveElement : document.body; },
});
const dispatchFocusEvent = (target, type, bubbles, relatedTarget) => {
  target.dispatchEvent(new FocusEvent(type, {
    bubbles, cancelable: false, composed: true, relatedTarget: relatedTarget ?? null, __trusted: true,
  }));
};
Object.defineProperties(HTMLElement.prototype, {
  offsetWidth: { configurable: true, get() { return browserBoxMetrics(this)?.offsetWidth ?? 0; } },
  offsetHeight: { configurable: true, get() { return browserBoxMetrics(this)?.offsetHeight ?? 0; } },
  offsetLeft: { configurable: true, get() { return browserBoxMetrics(this)?.offsetLeft ?? 0; } },
  offsetTop: { configurable: true, get() { return browserBoxMetrics(this)?.offsetTop ?? 0; } },
  offsetParent: {
    configurable: true,
    get() {
      const id = browserBoxMetrics(this)?.offsetParent;
      return id == null ? null : nativeNodes.get(id) ?? null;
    },
  },
});

// Native input is translated into normal LinkeDOM events first. Default
// actions are committed only when the corresponding cancelable event was not
// prevented, keeping JavaScript-visible DOM state authoritative.
const defineEventValues = (event, values) => {
  for (const [name, value] of Object.entries(values)) {
    Object.defineProperty(event, name, { value, enumerable: true, configurable: true });
  }
};
class UIEvent extends Event {
  constructor(type, init = {}) {
    super(type, init);
    defineEventValues(this, {
      view: init.view ?? window,
      detail: Number(init.detail ?? 0),
      isTrusted: Boolean(init.__trusted),
    });
  }
}
class MouseEvent extends UIEvent {
  constructor(type, init = {}) {
    super(type, init);
    const clientX = Number(init.clientX ?? 0);
    const clientY = Number(init.clientY ?? 0);
    defineEventValues(this, {
      screenX: Number(init.screenX ?? clientX), screenY: Number(init.screenY ?? clientY),
      clientX, clientY, pageX: Number(init.pageX ?? clientX), pageY: Number(init.pageY ?? clientY),
      offsetX: Number(init.offsetX ?? clientX), offsetY: Number(init.offsetY ?? clientY),
      movementX: Number(init.movementX ?? 0), movementY: Number(init.movementY ?? 0),
      ctrlKey: Boolean(init.ctrlKey), shiftKey: Boolean(init.shiftKey),
      altKey: Boolean(init.altKey), metaKey: Boolean(init.metaKey),
      button: Number(init.button ?? 0), buttons: Number(init.buttons ?? 0),
      relatedTarget: init.relatedTarget ?? null,
    });
  }
  getModifierState(key) {
    return Boolean({ Control: this.ctrlKey, Shift: this.shiftKey, Alt: this.altKey, Meta: this.metaKey }[key]);
  }
}
class PointerEvent extends MouseEvent {
  constructor(type, init = {}) {
    super(type, init);
    defineEventValues(this, {
      pointerId: Number(init.pointerId ?? 0), width: Number(init.width ?? 1),
      height: Number(init.height ?? 1), pressure: Number(init.pressure ?? 0),
      tangentialPressure: Number(init.tangentialPressure ?? 0),
      tiltX: Number(init.tiltX ?? 0), tiltY: Number(init.tiltY ?? 0),
      twist: Number(init.twist ?? 0), altitudeAngle: Number(init.altitudeAngle ?? Math.PI / 2),
      azimuthAngle: Number(init.azimuthAngle ?? 0), pointerType: String(init.pointerType ?? ''),
      isPrimary: Boolean(init.isPrimary),
    });
  }
  getCoalescedEvents() { return [this]; }
  getPredictedEvents() { return []; }
}
class WheelEvent extends MouseEvent {
  constructor(type, init = {}) {
    super(type, init);
    defineEventValues(this, {
      deltaX: Number(init.deltaX ?? 0), deltaY: Number(init.deltaY ?? 0),
      deltaZ: Number(init.deltaZ ?? 0), deltaMode: Number(init.deltaMode ?? 0),
    });
  }
}
class KeyboardEvent extends UIEvent {
  constructor(type, init = {}) {
    super(type, init);
    defineEventValues(this, {
      key: String(init.key ?? ''), code: String(init.code ?? ''),
      location: Number(init.location ?? 0), repeat: Boolean(init.repeat),
      isComposing: Boolean(init.isComposing), charCode: Number(init.charCode ?? 0),
      keyCode: Number(init.keyCode ?? 0), which: Number(init.which ?? init.keyCode ?? 0),
      ctrlKey: Boolean(init.ctrlKey), shiftKey: Boolean(init.shiftKey),
      altKey: Boolean(init.altKey), metaKey: Boolean(init.metaKey),
    });
  }
  getModifierState(key) {
    return Boolean({ Control: this.ctrlKey, Shift: this.shiftKey, Alt: this.altKey, Meta: this.metaKey }[key]);
  }
}
class FocusEvent extends UIEvent {
  constructor(type, init = {}) {
    super(type, init);
    defineEventValues(this, { relatedTarget: init.relatedTarget ?? null });
  }
}
globalThis.UIEvent = UIEvent;
globalThis.MouseEvent = MouseEvent;
globalThis.PointerEvent = PointerEvent;
globalThis.WheelEvent = WheelEvent;
globalThis.KeyboardEvent = KeyboardEvent;
globalThis.FocusEvent = FocusEvent;

const pointerCaptures = new Map();
const pointerDownTargets = new Map();
const suppressedCompatibilityPointers = new Set();
const pointerHoverTargets = new Map();
const pointerPositions = new Map();
const disabledControl = (element) =>
  element instanceof Element && element.hasAttribute('disabled') &&
  /^(button|input|select|textarea|option|optgroup|fieldset)$/.test(element.localName);
const eventCoordinatesFor = (target, init) => {
  const clientX = Number(init.clientX ?? init.x ?? 0);
  const clientY = Number(init.clientY ?? init.y ?? 0);
  const rect = target?.isConnected ? target.getBoundingClientRect() : new DOMRect();
  return { ...init, clientX, clientY, offsetX: clientX - rect.left, offsetY: clientY - rect.top };
};
const dispatchMouseLike = (target, Constructor, type, init = {}) => {
  if (!target) return null;
  const event = new Constructor(type, {
    bubbles: init.bubbles ?? true,
    cancelable: init.cancelable ?? true,
    composed: init.composed ?? true,
    ...eventCoordinatesFor(target, init),
  });
  target.dispatchEvent(event);
  return event;
};
const setControlChecked = (input, checked) => { input.checked = checked; };
const dispatchInputChange = (control) => {
  control.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
  control.dispatchEvent(new Event('change', { bubbles: true }));
};
const activationTargetFor = (target) => target instanceof Element
  ? target.closest('label,input,button,option,summary,a[href]')
  : null;
const commitClickDefault = (eventTarget) => {
  const target = activationTargetFor(eventTarget);
  if (!target || disabledControl(target)) return;
  if (target.localName === 'label') {
    const control = target.control || (target.htmlFor ? document.getElementById(target.htmlFor) : target.querySelector('input,button,select,textarea'));
    if (control && control !== target) dispatchClick(control);
    return;
  }
  if (target instanceof HTMLInputElement) {
    const type = String(target.type || 'text').toLowerCase();
    if (type === 'checkbox') {
      setControlChecked(target, !target.checked);
      dispatchInputChange(target);
    } else if (type === 'radio' && !target.checked) {
      const form = target.form;
      for (const radio of document.querySelectorAll('input[type="radio"]')) {
        if (radio !== target && radio.name === target.name && radio.form === form) setControlChecked(radio, false);
      }
      setControlChecked(target, true);
      dispatchInputChange(target);
    }
  }
  if (target.localName === 'option') {
    const select = target.closest('select');
    if (select) {
      if (!select.multiple) {
        for (const option of select.options) option.removeAttribute('selected');
      }
      target.setAttribute('selected', '');
      dispatchInputChange(select);
    }
  }
  if (target.localName === 'summary') {
    const details = target.parentElement?.localName === 'details' ? target.parentElement : null;
    if (details) {
      details.toggleAttribute('open');
      details.dispatchEvent(new Event('toggle'));
    }
  }
  const buttonType = target.localName === 'button'
    ? String(target.getAttribute('type') || 'submit').toLowerCase()
    : target instanceof HTMLInputElement ? String(target.type).toLowerCase() : '';
  const form = target.form || target.closest?.('form');
  if (form && (buttonType === 'submit' || buttonType === 'image')) {
    const submit = new Event('submit', { bubbles: true, cancelable: true });
    Object.defineProperty(submit, 'submitter', { value: target });
    form.dispatchEvent(submit);
  } else if (form && buttonType === 'reset') {
    const reset = new Event('reset', { bubbles: true, cancelable: true });
    if (form.dispatchEvent(reset)) {
      for (const control of form.querySelectorAll('input,textarea,select')) {
        if (control instanceof HTMLInputElement) {
          control.value = control.getAttribute('value') ?? '';
          setControlChecked(control, control.hasAttribute('checked'));
        } else if (control.localName === 'textarea') {
          control.value = control.textContent;
        } else {
          const selected = Array.from(control.options).filter((option) => option.hasAttribute('selected'));
          if (selected.length === 0 && control.options[0]) control.options[0].setAttribute('selected', '');
        }
      }
    }
  }
};
const dispatchClick = (target, init = {}) => {
  if (!target || disabledControl(activationTargetFor(target))) return false;
  const event = dispatchMouseLike(target, MouseEvent, 'click', {
    bubbles: true, cancelable: true, button: 0, buttons: 0, detail: init.detail ?? 1, ...init,
  });
  if (!event.defaultPrevented) commitClickDefault(target);
  return !event.defaultPrevented;
};
HTMLElement.prototype.click = function click() { dispatchClick(this, { detail: 0 }); };

Element.prototype.setPointerCapture = function setPointerCapture(pointerId) {
  pointerId = Number(pointerId);
  if (!Number.isInteger(pointerId) || !pointerDownTargets.has(pointerId)) {
    throw new DOMException('Pointer is not active', 'NotFoundError');
  }
  const previous = pointerCaptures.get(pointerId);
  if (previous === this) return;
  if (previous) dispatchMouseLike(previous, PointerEvent, 'lostpointercapture', { pointerId, bubbles: true, cancelable: false });
  pointerCaptures.set(pointerId, this);
  dispatchMouseLike(this, PointerEvent, 'gotpointercapture', { pointerId, bubbles: true, cancelable: false });
};
Element.prototype.releasePointerCapture = function releasePointerCapture(pointerId) {
  pointerId = Number(pointerId);
  if (pointerCaptures.get(pointerId) !== this) return;
  pointerCaptures.delete(pointerId);
  dispatchMouseLike(this, PointerEvent, 'lostpointercapture', { pointerId, bubbles: true, cancelable: false });
};
Element.prototype.hasPointerCapture = function hasPointerCapture(pointerId) {
  return pointerCaptures.get(Number(pointerId)) === this;
};

const nearestCommonElement = (a, b) => {
  const ancestors = new Set();
  for (let node = a; node instanceof Element; node = node.parentElement) ancestors.add(node);
  for (let node = b; node instanceof Element; node = node.parentElement) {
    if (ancestors.has(node)) return node;
  }
  return null;
};
const updatePointerHover = (pointerId, target, init) => {
  const previous = pointerHoverTargets.get(pointerId) ?? null;
  if (previous === target) return;
  if (previous) {
    dispatchMouseLike(previous, PointerEvent, 'pointerout', { ...init, pointerId, relatedTarget: target });
    dispatchMouseLike(previous, PointerEvent, 'pointerleave', { ...init, pointerId, relatedTarget: target, bubbles: false, cancelable: false });
    if (init.pointerType === 'mouse') {
      dispatchMouseLike(previous, MouseEvent, 'mouseout', { ...init, relatedTarget: target });
      dispatchMouseLike(previous, MouseEvent, 'mouseleave', { ...init, relatedTarget: target, bubbles: false, cancelable: false });
    }
  }
  if (target) {
    dispatchMouseLike(target, PointerEvent, 'pointerover', { ...init, pointerId, relatedTarget: previous });
    dispatchMouseLike(target, PointerEvent, 'pointerenter', { ...init, pointerId, relatedTarget: previous, bubbles: false, cancelable: false });
    if (init.pointerType === 'mouse') {
      dispatchMouseLike(target, MouseEvent, 'mouseover', { ...init, relatedTarget: previous });
      dispatchMouseLike(target, MouseEvent, 'mouseenter', { ...init, relatedTarget: previous, bubbles: false, cancelable: false });
    }
    pointerHoverTargets.set(pointerId, target);
  } else {
    pointerHoverTargets.delete(pointerId);
  }
};
globalThis.__dispatchBrowserPointerEvent = (type, init = {}) => {
  type = String(type);
  syncBrowserDocument();
  const pointerId = Number(init.pointerId ?? 1);
  const pointerType = String(init.pointerType ?? 'mouse');
  const previousPosition = pointerPositions.get(pointerId);
  const clientX = Number(init.clientX ?? init.x ?? previousPosition?.x ?? 0);
  const clientY = Number(init.clientY ?? init.y ?? previousPosition?.y ?? 0);
  pointerPositions.set(pointerId, { x: clientX, y: clientY });
  const captured = pointerCaptures.get(pointerId);
  // Pointer Lock retargets relative motion to the locked element regardless of
  // the hidden cursor's last hit-test position.
  const target = __pointerLockElement?.isConnected ? __pointerLockElement
    : captured?.isConnected ? captured : document.elementFromPoint(clientX, clientY);
  const button = Number(init.button ?? (type === 'pointermove' ? -1 : 0));
  const defaultButtons = type === 'pointerdown' ? 1 << Math.max(0, button)
    : type === 'pointerup' || type === 'pointercancel' ? 0 : pointerDownTargets.has(pointerId) ? 1 : 0;
  const eventInit = {
    ...init, __trusted: true, pointerId, pointerType, clientX, clientY, button,
    buttons: Number(init.buttons ?? defaultButtons),
    movementX: Number(init.movementX ?? (previousPosition ? clientX - previousPosition.x : 0)),
    movementY: Number(init.movementY ?? (previousPosition ? clientY - previousPosition.y : 0)),
    pressure: Number(init.pressure ?? (defaultButtons ? 0.5 : 0)),
    isPrimary: init.isPrimary ?? pointerId === 1,
  };
  if (type === 'pointermove') {
    if (Deno.core.ops.op_browser_set_pointer_state(0, clientX, clientY)) scheduleBrowserObservers();
    if (!captured) updatePointerHover(pointerId, target, eventInit);
  } else if (type === 'pointerdown') {
    if (Deno.core.ops.op_browser_set_pointer_state(1, clientX, clientY)) scheduleBrowserObservers();
    if (!captured) updatePointerHover(pointerId, target, eventInit);
  }
  if (!target) {
    if (type === 'pointerup' || type === 'pointercancel') {
      if (Deno.core.ops.op_browser_set_pointer_state(2, clientX, clientY)) scheduleBrowserObservers();
      pointerDownTargets.delete(pointerId);
      suppressedCompatibilityPointers.delete(pointerId);
    }
    return false;
  }
  // A pointer becomes active before pointerdown listeners run, allowing those
  // listeners (including OrbitControls) to capture it during dispatch.
  if (type === 'pointerdown') pointerDownTargets.set(pointerId, target);
  const pointerEvent = dispatchMouseLike(target, PointerEvent, type, eventInit);
  if (type === 'pointerdown') {
    if (pointerEvent.defaultPrevented) suppressedCompatibilityPointers.add(pointerId);
    else suppressedCompatibilityPointers.delete(pointerId);
    if (!pointerEvent.defaultPrevented && pointerType === 'mouse') {
      const mouse = dispatchMouseLike(target, MouseEvent, 'mousedown', eventInit);
      const focusTarget = target.closest?.('button,input,select,textarea,a[href],[tabindex],summary') ?? target;
      if (!mouse.defaultPrevented && focusTarget instanceof HTMLElement && !disabledControl(focusTarget) && isBrowserFocusable(focusTarget)) focusTarget.focus();
    }
  } else if (type === 'pointermove' && pointerType === 'mouse' && !pointerEvent.defaultPrevented) {
    dispatchMouseLike(target, MouseEvent, 'mousemove', eventInit);
  } else if (type === 'pointerup') {
    if (Deno.core.ops.op_browser_set_pointer_state(2, clientX, clientY)) scheduleBrowserObservers();
    if (!pointerEvent.defaultPrevented && !suppressedCompatibilityPointers.has(pointerId)) {
      const clickTarget = nearestCommonElement(pointerDownTargets.get(pointerId), target);
      if (pointerType === 'mouse') dispatchMouseLike(target, MouseEvent, 'mouseup', eventInit);
      if (clickTarget && (pointerType === 'mouse' || eventInit.isPrimary)) dispatchClick(clickTarget, eventInit);
    }
    pointerDownTargets.delete(pointerId);
    suppressedCompatibilityPointers.delete(pointerId);
    const capture = pointerCaptures.get(pointerId);
    if (capture) {
      capture.releasePointerCapture(pointerId);
      updatePointerHover(pointerId, document.elementFromPoint(clientX, clientY), eventInit);
    }
    if (pointerType !== 'mouse') {
      updatePointerHover(pointerId, null, eventInit);
      if (Deno.core.ops.op_browser_set_pointer_state(0, -1, -1)) scheduleBrowserObservers();
    }
  } else if (type === 'pointercancel') {
    if (Deno.core.ops.op_browser_set_pointer_state(2, clientX, clientY)) scheduleBrowserObservers();
    pointerDownTargets.delete(pointerId);
    suppressedCompatibilityPointers.delete(pointerId);
    const capture = pointerCaptures.get(pointerId);
    if (capture) capture.releasePointerCapture(pointerId);
    updatePointerHover(pointerId, null, eventInit);
    if (Deno.core.ops.op_browser_set_pointer_state(0, -1, -1)) scheduleBrowserObservers();
  }
  return !pointerEvent.defaultPrevented;
};

globalThis.__dispatchBrowserWheelEvent = (init = {}) => {
  syncBrowserDocument();
  const clientX = Number(init.clientX ?? init.x ?? 0);
  const clientY = Number(init.clientY ?? init.y ?? 0);
  const target = document.elementFromPoint(clientX, clientY);
  if (!target) return false;
  const event = dispatchMouseLike(target, WheelEvent, 'wheel', { ...init, __trusted: true, clientX, clientY });
  if (!event.defaultPrevented) {
    const multiplier = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? innerHeight : 1;
    let scroller = target;
    while (scroller instanceof Element) {
      if (scroller.scrollHeight > scroller.clientHeight || scroller.scrollWidth > scroller.clientWidth) {
        scroller.scrollBy(event.deltaX * multiplier, event.deltaY * multiplier);
        break;
      }
      scroller = scroller.parentElement;
    }
  }
  return !event.defaultPrevented;
};
const tabbableElements = () => Array.from(document.querySelectorAll(
  'a[href],button,input,select,textarea,[tabindex]'
)).filter((element) => element instanceof HTMLElement && !disabledControl(element) && browserTabIndex(element) >= 0)
  .sort((a, b) => {
    const at = browserTabIndex(a); const bt = browserTabIndex(b);
    if (at > 0 && bt === 0) return -1;
    if (at === 0 && bt > 0) return 1;
    return at > 0 && bt > 0 ? at - bt : 0;
  });
const pendingSpaceActivations = new WeakSet();
globalThis.__dispatchBrowserKeyboardEvent = (type, init = {}) => {
  const target = document.activeElement ?? document.body;
  const event = new KeyboardEvent(String(type), { bubbles: true, cancelable: true, composed: true, ...init, __trusted: true });
  target.dispatchEvent(event);
  if (type === 'keydown' && !event.defaultPrevented) {
    if (event.key === 'Tab') {
      const elements = tabbableElements();
      if (elements.length !== 0) {
        const current = elements.indexOf(document.activeElement);
        const delta = event.shiftKey ? -1 : 1;
        elements[(current + delta + elements.length) % elements.length].focus();
      }
    } else if (event.key === 'Enter' && /^(button|input)$/.test(target.localName)) {
      dispatchClick(target, { detail: 0 });
    } else if (event.key === ' ' && /^(button|input)$/.test(target.localName)) {
      pendingSpaceActivations.add(target);
    }
  } else if (type === 'keyup' && event.key === ' ' && pendingSpaceActivations.has(target)) {
    pendingSpaceActivations.delete(target);
    if (!event.defaultPrevented) dispatchClick(target, { detail: 0 });
  }
  return !event.defaultPrevented;
};

const inputCheckedState = new WeakMap();
Object.defineProperties(HTMLInputElement.prototype, {
  checked: {
    configurable: true,
    enumerable: true,
    get() { return inputCheckedState.get(this) ?? this.hasAttribute('checked'); },
    set(value) {
      const checked = Boolean(value);
      if (checked === this.checked) return;
      inputCheckedState.set(this, checked);
      markBrowserDocumentDirty();
    },
  },
  defaultChecked: {
    configurable: true,
    enumerable: true,
    get() { return this.hasAttribute('checked'); },
    set(value) { this.toggleAttribute('checked', Boolean(value)); },
  },
});

Object.defineProperty(HTMLSelectElement.prototype, 'value', {
  configurable: true,
  enumerable: true,
  get() {
    return this.querySelector('option[selected]')?.value ?? this.options[0]?.value ?? '';
  },
  set(value) {
    value = String(value);
    const options = Array.from(this.options);
    for (const option of options) option.removeAttribute('selected');
    const match = options.find((option) => option.value === value);
    if (match) match.setAttribute('selected', '');
  },
});

// Browser globals LinkeDOM doesn't own.
globalThis.navigator = globalThis.navigator || {};
globalThis.performance = globalThis.performance || { now: () => 0, _now: () => 0 };
globalThis.devicePixelRatio = Number(globalThis.__devicePixelRatio) || 1;
delete globalThis.__devicePixelRatio;
// three.js e2e renders at viewScale 2 and downsamples to the 400×250
// reference image. Keep CSS viewport dimensions aligned with that harness.
globalThis.innerWidth = globalThis.__viewportWidth || 800;
globalThis.innerHeight = globalThis.__viewportHeight || 500;

const mediaQueryLists = new Set();
const evaluateMediaQuery = (query) => {
  syncBrowserDocument();
  return Deno.core.ops.op_browser_media_query_matches(String(query));
};
class MediaQueryList extends EventTarget {
  constructor(media) {
    super();
    this.media = String(media);
    this.onchange = null;
    this._matches = evaluateMediaQuery(this.media);
    mediaQueryLists.add(this);
  }
  get matches() { return this._matches; }
  addListener(callback) { this.addEventListener('change', callback); }
  removeListener(callback) { this.removeEventListener('change', callback); }
  _evaluate() {
    const matches = Deno.core.ops.op_browser_media_query_matches(this.media);
    if (matches === this._matches) return;
    this._matches = matches;
    const event = new Event('change');
    Object.defineProperties(event, {
      matches: { value: matches, enumerable: true },
      media: { value: this.media, enumerable: true },
    });
    this.dispatchEvent(event);
    if (typeof this.onchange === 'function') this.onchange.call(this, event);
  }
}
globalThis.MediaQueryList = MediaQueryList;
globalThis.matchMedia = (query) => new MediaQueryList(query);
const computedPropertyName = (name) => {
  name = String(name);
  if (name === 'cssFloat' || name === 'styleFloat') return 'float';
  if (name.startsWith('--')) return name;
  return name.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`).toLowerCase();
};
class ComputedStyleDeclaration {
  constructor(element, pseudo) {
    this.element = element;
    this.pseudo = pseudo;
  }
  getPropertyValue(name) {
    if (!this.element.isConnected) return '';
    syncBrowserDocument();
    return Deno.core.ops.op_browser_computed_property(
      ensureNativeNodeId(this.element),
      computedPropertyName(name),
      this.pseudo || '',
    );
  }
  getPropertyPriority(_name) { return ''; }
  item(_index) { return ''; }
  setProperty() {}
  removeProperty() { return ''; }
  get cssText() { return ''; }
  set cssText(_value) {}
  get length() { return 0; }
  get parentRule() { return null; }
}
globalThis.getComputedStyle = (element, pseudo = null) => {
  if (!(element instanceof Element)) throw new TypeError('getComputedStyle requires an Element');
  if (pseudo !== null && pseudo !== '' && pseudo !== '::before' && pseudo !== '::after') {
    throw new TypeError(`Unsupported pseudo-element ${pseudo}`);
  }
  const declaration = new ComputedStyleDeclaration(element, pseudo || null);
  return new Proxy(declaration, {
    get(target, property, receiver) {
      if (typeof property !== 'string' || Reflect.has(target, property)) {
        return Reflect.get(target, property, receiver);
      }
      return target.getPropertyValue(property);
    },
    set() { return true; },
  });
};

// Layout observers are delivered from the same resolved Blitz document used by
// synchronous geometry APIs. Delivery is microtask-scheduled and coalesced;
// observer callbacks never run in the middle of reconciliation.
const resizeObservers = new Set();
const intersectionObservers = new Set();
let observerDeliveryScheduled = false;
let observerDeliveryActive = false;
const nativeComputedValue = (element, property) =>
  Deno.core.ops.op_browser_computed_property(ensureNativeNodeId(element), property, '');
const nativeComputedNumber = (element, property) => {
  const value = nativeComputedValue(element, property);
  const number = Number.parseFloat(value);
  return Number.isFinite(number) ? number : 0;
};
class ResizeObserverSize {
  constructor(inlineSize, blockSize) {
    this.inlineSize = inlineSize;
    this.blockSize = blockSize;
  }
}
class ResizeObserverEntry {
  constructor(target, contentRect, contentBoxSize, borderBoxSize, devicePixelContentBoxSize) {
    this.target = target;
    this.contentRect = contentRect;
    this.contentBoxSize = [contentBoxSize];
    this.borderBoxSize = [borderBoxSize];
    this.devicePixelContentBoxSize = [devicePixelContentBoxSize];
  }
}
const resizeEntryFor = (target) => {
  const metrics = Deno.core.ops.op_browser_box_metrics(ensureNativeNodeId(target));
  const paddingLeft = nativeComputedNumber(target, 'padding-left');
  const paddingRight = nativeComputedNumber(target, 'padding-right');
  const paddingTop = nativeComputedNumber(target, 'padding-top');
  const paddingBottom = nativeComputedNumber(target, 'padding-bottom');
  const contentWidth = Math.max(0, metrics.clientWidth - paddingLeft - paddingRight);
  const contentHeight = Math.max(0, metrics.clientHeight - paddingTop - paddingBottom);
  const contentRect = new DOMRectReadOnly(
    metrics.clientLeft + paddingLeft,
    metrics.clientTop + paddingTop,
    contentWidth,
    contentHeight,
  );
  const vertical = /^(vertical|sideways)/.test(nativeComputedValue(target, 'writing-mode'));
  const logicalSize = (width, height) => new ResizeObserverSize(
    vertical ? height : width,
    vertical ? width : height,
  );
  return new ResizeObserverEntry(
    target,
    contentRect,
    logicalSize(contentWidth, contentHeight),
    logicalSize(metrics.offsetWidth, metrics.offsetHeight),
    logicalSize(contentWidth * devicePixelRatio, contentHeight * devicePixelRatio),
  );
};
class ResizeObserver {
  constructor(callback) {
    if (typeof callback !== 'function') throw new TypeError('ResizeObserver callback must be a function');
    this._callback = callback;
    this._observations = new Map();
    this._queued = [];
    resizeObservers.add(this);
  }
  observe(target, options = {}) {
    if (!(target instanceof Element)) throw new TypeError('ResizeObserver target must be an Element');
    const box = options.box ?? 'content-box';
    if (!['content-box', 'border-box', 'device-pixel-content-box'].includes(box)) {
      throw new TypeError(`Invalid ResizeObserver box ${box}`);
    }
    this._observations.set(target, { box, size: null });
    scheduleBrowserObservers();
  }
  unobserve(target) { this._observations.delete(target); }
  disconnect() { this._observations.clear(); this._queued.length = 0; }
  _gather() {
    for (const [target, observation] of this._observations) {
      if (!target.isConnected) continue;
      const entry = resizeEntryFor(target);
      const size = observation.box === 'border-box'
        ? entry.borderBoxSize[0]
        : observation.box === 'device-pixel-content-box'
          ? entry.devicePixelContentBoxSize[0]
          : entry.contentBoxSize[0];
      const signature = `${size.inlineSize}:${size.blockSize}`;
      if (signature !== observation.size) {
        observation.size = signature;
        this._queued.push(entry);
      }
    }
  }
  _deliver() {
    if (this._queued.length === 0) return;
    const entries = this._queued.splice(0);
    this._callback.call(this, entries, this);
  }
}

const parseRootMargin = (value) => {
  const parts = String(value ?? '0px').trim().split(/\s+/);
  if (parts.length < 1 || parts.length > 4) throw new SyntaxError('Invalid rootMargin');
  const parsed = parts.map((part) => {
    const match = /^(-?(?:\d+|\d*\.\d+))(px|%)?$/.exec(part);
    if (!match || (!match[2] && Number(match[1]) !== 0)) {
      throw new SyntaxError(`Invalid rootMargin component ${part}`);
    }
    return { value: Number(match[1]), unit: match[2] || 'px' };
  });
  if (parsed.length === 1) return [parsed[0], parsed[0], parsed[0], parsed[0]];
  if (parsed.length === 2) return [parsed[0], parsed[1], parsed[0], parsed[1]];
  if (parsed.length === 3) return [parsed[0], parsed[1], parsed[2], parsed[1]];
  return parsed;
};
class IntersectionObserverEntry {
  constructor(init) { Object.assign(this, init); }
}
class IntersectionObserver {
  constructor(callback, options = {}) {
    if (typeof callback !== 'function') throw new TypeError('IntersectionObserver callback must be a function');
    const root = options.root ?? null;
    if (root !== null && root !== document && !(root instanceof Element)) {
      throw new TypeError('IntersectionObserver root must be an Element, Document, or null');
    }
    let thresholds = options.threshold ?? 0;
    if (!Array.isArray(thresholds)) thresholds = [thresholds];
    thresholds = Array.from(new Set(thresholds.map(Number))).sort((a, b) => a - b);
    if (thresholds.some((value) => !Number.isFinite(value) || value < 0 || value > 1)) {
      throw new RangeError('IntersectionObserver threshold must be between 0 and 1');
    }
    this.root = root;
    this.rootMargin = String(options.rootMargin ?? '0px');
    this.thresholds = thresholds.length === 0 ? [0] : thresholds;
    this._margins = parseRootMargin(this.rootMargin);
    this._callback = callback;
    this._observations = new Map();
    this._queued = [];
    intersectionObservers.add(this);
  }
  observe(target) {
    if (!(target instanceof Element)) throw new TypeError('IntersectionObserver target must be an Element');
    if (!this._observations.has(target)) this._observations.set(target, null);
    scheduleBrowserObservers();
  }
  unobserve(target) { this._observations.delete(target); }
  disconnect() { this._observations.clear(); this._queued.length = 0; }
  takeRecords() { return this._queued.splice(0); }
  _gather() {
    const rootElement = this.root instanceof Element ? this.root : null;
    const rootWidth = rootElement?.getBoundingClientRect().width ?? innerWidth;
    const margins = this._margins.map((margin) =>
      margin.unit === '%' ? margin.value * rootWidth / 100 : margin.value
    );
    for (const [target, previous] of this._observations) {
      if (!target.isConnected || (rootElement && !rootElement.isConnected)) continue;
      const targetRect = Deno.core.ops.op_browser_rect(ensureNativeNodeId(target));
      const boundingClientRect = new DOMRectReadOnly(
        targetRect.x, targetRect.y, targetRect.width, targetRect.height,
      );
      const native = Deno.core.ops.op_browser_intersection(
        ensureNativeNodeId(target),
        rootElement ? ensureNativeNodeId(rootElement) : 0,
        margins[0], margins[1], margins[2], margins[3],
      );
      const intersectionRect = DOMRectReadOnly.from(native.intersectionRect);
      const rootBounds = DOMRectReadOnly.from(native.rootBounds);
      const targetArea = Math.max(0, boundingClientRect.width * boundingClientRect.height);
      const intersectionArea = Math.max(0, intersectionRect.width * intersectionRect.height);
      const isIntersecting = intersectionArea > 0;
      const intersectionRatio = targetArea === 0 ? (isIntersecting ? 1 : 0) : intersectionArea / targetArea;
      const thresholdIndex = this.thresholds.findIndex((threshold) => threshold > intersectionRatio);
      const state = `${thresholdIndex}:${isIntersecting}`;
      if (state === previous) continue;
      this._observations.set(target, state);
      this._queued.push(new IntersectionObserverEntry({
        time: performance.now(), target, rootBounds, boundingClientRect,
        intersectionRect, isIntersecting, intersectionRatio,
      }));
    }
  }
  _deliver() {
    const entries = this.takeRecords();
    if (entries.length !== 0) this._callback.call(this, entries, this);
  }
}
globalThis.ResizeObserver = ResizeObserver;
globalThis.ResizeObserverEntry = ResizeObserverEntry;
globalThis.ResizeObserverSize = ResizeObserverSize;
globalThis.IntersectionObserver = IntersectionObserver;
globalThis.IntersectionObserverEntry = IntersectionObserverEntry;

scheduleBrowserObservers = () => {
  if (observerDeliveryScheduled || observerDeliveryActive) return;
  const needed = mediaQueryLists.size !== 0 ||
    Array.from(resizeObservers).some((observer) => observer._observations.size !== 0) ||
    Array.from(intersectionObservers).some((observer) => observer._observations.size !== 0);
  if (!needed) return;
  observerDeliveryScheduled = true;
  queueMicrotask(() => {
    observerDeliveryScheduled = false;
    observerDeliveryActive = true;
    try {
      syncBrowserDocument();
      for (const list of mediaQueryLists) list._evaluate();
      for (const observer of resizeObservers) observer._gather();
      for (const observer of intersectionObservers) observer._gather();
      const errors = [];
      for (const observer of resizeObservers) {
        try { observer._deliver(); } catch (error) { errors.push(error); }
      }
      for (const observer of intersectionObservers) {
        try { observer._deliver(); } catch (error) { errors.push(error); }
      }
      if (errors.length !== 0) queueMicrotask(() => { throw errors[0]; });
    } finally {
      observerDeliveryActive = false;
    }
  });
};

// Web Storage is not implemented by LinkeDOM. Provide the synchronous Storage
// API with separate per-document local/session stores. Property access is
// supported as required by Web Storage's named-property behavior.
const storageConstructorKey = Symbol('Storage constructor key');
const storageItems = new WeakMap();
const itemsFor = (storage) => {
  const items = storageItems.get(storage);
  if (!items) throw new TypeError('Illegal invocation');
  return items;
};

class Storage {
  constructor(key) {
    if (key !== storageConstructorKey) throw new TypeError('Illegal constructor');
    storageItems.set(this, new Map());
  }

  get length() { return itemsFor(this).size; }
  key(index) { return Array.from(itemsFor(this).keys())[Number(index)] ?? null; }
  getItem(key) { return itemsFor(this).get(String(key)) ?? null; }
  setItem(key, value) { itemsFor(this).set(String(key), String(value)); }
  removeItem(key) { itemsFor(this).delete(String(key)); }
  clear() { itemsFor(this).clear(); }
}

const createStorage = () => {
  const target = new Storage(storageConstructorKey);
  const proxy = new Proxy(target, {
    get(target, property, receiver) {
      if (typeof property === 'string' && !(property in target)) {
        return target.getItem(property) ?? undefined;
      }
      return Reflect.get(target, property, receiver);
    },
    set(target, property, value, receiver) {
      if (typeof property === 'string' && !(property in target)) {
        target.setItem(property, value);
        return true;
      }
      return Reflect.set(target, property, value, receiver);
    },
    deleteProperty(target, property) {
      if (typeof property === 'string' && !(property in target)) {
        target.removeItem(property);
        return true;
      }
      return Reflect.deleteProperty(target, property);
    },
    has(target, property) {
      return Reflect.has(target, property) ||
        (typeof property === 'string' && target.getItem(property) !== null);
    },
    ownKeys(target) {
      return [...Reflect.ownKeys(target), ...itemsFor(target).keys()];
    },
    getOwnPropertyDescriptor(target, property) {
      const descriptor = Reflect.getOwnPropertyDescriptor(target, property);
      if (descriptor || typeof property !== 'string' || target.getItem(property) === null) {
        return descriptor;
      }
      return { value: target.getItem(property), writable: true, enumerable: true, configurable: true };
    },
  });
  storageItems.set(proxy, storageItems.get(target));
  return proxy;
};

globalThis.Storage = globalThis.Storage || Storage;
globalThis.localStorage = globalThis.localStorage || createStorage();
globalThis.sessionStorage = globalThis.sessionStorage || createStorage();

// Native asset fetch. The game runtime supports GET/HEAD against its file URL
// space; unsupported methods and schemes reject rather than pretending to work.
const normalizeHeaderName = (name) => String(name).trim().toLowerCase();
const normalizeHeaderValue = (value) => String(value).trim();

class Headers {
  #values = new Map();

  constructor(init = undefined) {
    if (init instanceof Headers) {
      for (const [name, value] of init) this.#values.set(name, value);
    } else if (init != null && typeof init[Symbol.iterator] === 'function') {
      for (const pair of init) {
        if (!pair || pair.length !== 2) throw new TypeError('Each header pair must contain exactly two items');
        this.append(pair[0], pair[1]);
      }
    } else if (init != null && typeof init === 'object') {
      for (const [name, value] of Object.entries(init)) this.append(name, value);
    }
  }

  append(name, value) {
    name = normalizeHeaderName(name);
    value = normalizeHeaderValue(value);
    const previous = this.#values.get(name);
    this.#values.set(name, previous === undefined ? value : `${previous}, ${value}`);
  }
  delete(name) { this.#values.delete(normalizeHeaderName(name)); }
  get(name) { return this.#values.get(normalizeHeaderName(name)) ?? null; }
  has(name) { return this.#values.has(normalizeHeaderName(name)); }
  set(name, value) { this.#values.set(normalizeHeaderName(name), normalizeHeaderValue(value)); }
  entries() { return this.#values.entries(); }
  keys() { return this.#values.keys(); }
  values() { return this.#values.values(); }
  forEach(callback, thisArg = undefined) {
    for (const [name, value] of this.#values) callback.call(thisArg, value, name, this);
  }
  [Symbol.iterator]() { return this.entries(); }
}

const defaultSignal = new AbortController().signal;
class Request {
  constructor(input, init = {}) {
    const source = input instanceof Request ? input : null;
    const rawURL = source ? source.url : input;
    this.url = new URL(String(rawURL), globalThis.__exampleURL).href;
    this.method = String(init.method ?? source?.method ?? 'GET').toUpperCase();
    this.headers = new Headers(init.headers ?? source?.headers);
    this.signal = init.signal ?? source?.signal ?? defaultSignal;
    if (!(this.signal instanceof AbortSignal)) throw new TypeError('Request signal must be an AbortSignal');
    this.credentials = init.credentials ?? source?.credentials ?? 'same-origin';
    this.cache = init.cache ?? source?.cache ?? 'default';
    this.mode = init.mode ?? source?.mode ?? 'cors';
    this.redirect = init.redirect ?? source?.redirect ?? 'follow';
    this.referrer = init.referrer ?? source?.referrer ?? 'about:client';
    this.referrerPolicy = init.referrerPolicy ?? source?.referrerPolicy ?? '';
    this.integrity = init.integrity ?? source?.integrity ?? '';
    this.keepalive = Boolean(init.keepalive ?? source?.keepalive ?? false);
    this.destination = source?.destination ?? '';
    this.body = init.body ?? source?.body ?? null;
    this.bodyUsed = false;
  }

  clone() {
    if (this.bodyUsed) throw new TypeError('Cannot clone a used Request');
    return new Request(this);
  }
  async arrayBuffer() { return bodyArrayBuffer(this); }
  async text() { return new TextDecoder().decode(new Uint8Array(await this.arrayBuffer())); }
  async json() { return JSON.parse(await this.text()); }
}

const responseBytes = new WeakMap();
const consumeResponse = (response) => {
  if (response.bodyUsed) return Promise.reject(new TypeError('Body has already been consumed'));
  response.bodyUsed = true;
  return responseBytes.get(response).slice();
};

class Response {
  constructor(body = null, init = {}) {
    let bytes;
    if (body == null) bytes = new Uint8Array();
    else if (body instanceof Uint8Array) bytes = body.slice();
    else if (body instanceof ArrayBuffer) bytes = new Uint8Array(body.slice(0));
    else if (ArrayBuffer.isView(body)) bytes = new Uint8Array(body.buffer, body.byteOffset, body.byteLength).slice();
    else if (typeof body === 'string') bytes = new TextEncoder().encode(body);
    else throw new TypeError('Unsupported Response body type');
    responseBytes.set(this, bytes);
    this.status = Number(init.status ?? 200);
    this.statusText = String(init.statusText ?? '');
    this.headers = new Headers(init.headers);
    this.url = String(init.url ?? '');
    this.redirected = Boolean(init.redirected ?? false);
    this.type = init.type ?? 'basic';
    this.body = null;
    this.bodyUsed = false;
  }

  get ok() { return this.status >= 200 && this.status <= 299; }
  async arrayBuffer() {
    const bytes = await consumeResponse(this);
    return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
  }
  async bytes() { return consumeResponse(this); }
  async blob() { return new Blob([await consumeResponse(this)], { type: this.headers.get('content-type') ?? '' }); }
  async text() { return new TextDecoder().decode(await consumeResponse(this)); }
  async json() { return JSON.parse(await this.text()); }
  clone() {
    if (this.bodyUsed) throw new TypeError('Cannot clone a used Response');
    return new Response(responseBytes.get(this), {
      status: this.status, statusText: this.statusText, headers: this.headers,
      url: this.url, redirected: this.redirected, type: this.type,
    });
  }
  static error() { return new Response(null, { status: 0, type: 'error' }); }
  static json(value, init = {}) {
    const headers = new Headers(init.headers);
    if (!headers.has('content-type')) headers.set('content-type', 'application/json');
    return new Response(JSON.stringify(value), { ...init, headers });
  }
  static redirect(url, status = 302) {
    return new Response(null, { status, headers: { location: new URL(url, location.href).href } });
  }
}

const mimeTypes = new Map(Object.entries({
  png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', webp: 'image/webp', gif: 'image/gif',
  svg: 'image/svg+xml', json: 'application/json', gltf: 'model/gltf+json', glb: 'model/gltf-binary',
  bin: 'application/octet-stream', hdr: 'image/vnd.radiance', exr: 'image/x-exr', ktx2: 'image/ktx2',
  dds: 'image/vnd-ms.dds', js: 'text/javascript', mjs: 'text/javascript', css: 'text/css',
  html: 'text/html', txt: 'text/plain', wasm: 'application/wasm', ttf: 'font/ttf', woff: 'font/woff',
  woff2: 'font/woff2', mp3: 'audio/mpeg', ogg: 'audio/ogg', mp4: 'video/mp4', webm: 'video/webm',
}));
const mimeTypeFor = (url) => {
  const path = new URL(url).pathname;
  const extension = path.slice(path.lastIndexOf('.') + 1).toLowerCase();
  return mimeTypes.get(extension) ?? 'application/octet-stream';
};

const bodyArrayBuffer = async (body) => {
  if (body.bodyUsed) throw new TypeError('Body has already been consumed');
  body.bodyUsed = true;
  if (body.body == null) return new ArrayBuffer(0);
  if (body.body instanceof Uint8Array) return body.body.slice().buffer;
  if (body.body instanceof ArrayBuffer) return body.body.slice(0);
  if (typeof body.body === 'string') return new TextEncoder().encode(body.body).buffer;
  throw new TypeError('Unsupported body type');
};

globalThis.__pendingFetches = 0;
globalThis.__fetchActivity = 0;
globalThis.__loadedAssetBytes = 0;
const reportFetchState = () => {
  Deno.core.ops.op_set_fetch_state(globalThis.__pendingFetches, globalThis.__fetchActivity);
};
const fetch = async (input, init = undefined) => {
  globalThis.__pendingFetches++;
  globalThis.__fetchActivity++;
  reportFetchState();
  try {
  const request = new Request(input, init);
  request.signal.throwIfAborted();
  if (request.method !== 'GET' && request.method !== 'HEAD') {
    throw new TypeError(`Native asset fetch does not support ${request.method}`);
  }
  const parsed = new URL(request.url);
  let bytes;
  let contentType;
  if (parsed.protocol === 'file:') {
    bytes = request.method === 'HEAD' ? new Uint8Array() : await Deno.core.ops.op_fetch_url(request.url);
    contentType = mimeTypeFor(request.url);
  } else if (parsed.protocol === 'blob:') {
    const blob = globalThis.__blobFromObjectURL(request.url);
    if (blob === null) throw new TypeError(`Blob URL does not exist: ${request.url}`);
    bytes = request.method === 'HEAD' ? new Uint8Array() : new Uint8Array(await blob.arrayBuffer());
    contentType = blob.type || 'application/octet-stream';
  } else if (parsed.protocol === 'http:' || parsed.protocol === 'https:') {
    bytes = request.method === 'HEAD' ? new Uint8Array() : await Deno.core.ops.op_fetch_url(request.url);
    contentType = mimeTypeFor(request.url);
  } else {
    throw new TypeError(`Native asset fetch does not support ${parsed.protocol} URLs`);
  }
  request.signal.throwIfAborted();
  if (parsed.protocol !== 'blob:') {
    globalThis.__loadedAssetBytes += bytes.byteLength;
    Deno.core.ops.op_set_loaded_asset_bytes(Math.min(globalThis.__loadedAssetBytes, 0xffffffff));
  }
  return new Response(bytes, {
    status: 200,
    url: request.url,
    headers: {
      'content-type': contentType,
      'content-length': String(bytes.byteLength),
    },
  });
  } finally {
    globalThis.__pendingFetches--;
    globalThis.__fetchActivity++;
    reportFetchState();
  }
};

globalThis.Headers = Headers;
globalThis.Request = Request;
globalThis.Response = Response;
globalThis.fetch = fetch;

const flipBitmapY = (data, width, height) => {
  const stride = width * 4;
  const flipped = new Uint8Array(data.length);
  for (let y = 0; y < height; y++) {
    flipped.set(data.subarray((height - 1 - y) * stride, (height - y) * stride), y * stride);
  }
  return flipped;
};

class ImageBitmap {
  constructor(width, height, data) {
    this.width = width;
    this.height = height;
    this.data = data;
  }
  close() { this.data = new Uint8Array(); }
}

const createImageBitmap = async (source, ...args) => {
  let decoded;
  if (source?.data instanceof Uint8Array && Number.isFinite(source.width) && Number.isFinite(source.height)) {
    decoded = { width: source.width, height: source.height, data: source.data.slice() };
  } else if (source && typeof source.arrayBuffer === 'function') {
    decoded = Deno.core.ops.op_decode_image(new Uint8Array(await source.arrayBuffer()));
  } else {
    throw new TypeError('Unsupported ImageBitmap source');
  }
  let data = new Uint8Array(decoded.data);
  const options = args.length >= 4 ? (args[4] ?? {}) : (args[0] ?? {});
  if (options.imageOrientation === 'flipY') data = flipBitmapY(data, decoded.width, decoded.height);
  if (options.premultiplyAlpha === 'premultiply') {
    data = data.slice();
    for (let i = 0; i < data.length; i += 4) {
      const alpha = data[i + 3] / 255;
      data[i] = Math.round(data[i] * alpha);
      data[i + 1] = Math.round(data[i + 1] * alpha);
      data[i + 2] = Math.round(data[i + 2] * alpha);
    }
  }
  return new ImageBitmap(decoded.width, decoded.height, data);
};

globalThis.ImageBitmap = ImageBitmap;
globalThis.createImageBitmap = createImageBitmap;

// LinkeDOM models image elements but does not load their source. Attach native
// decode behavior to the real HTMLImageElement so three.js event handling and
// instanceof checks continue to use the DOM implementation.
const imageSource = Object.getOwnPropertyDescriptor(HTMLImageElement.prototype, 'src');
Object.defineProperty(HTMLImageElement.prototype, 'src', {
  configurable: true,
  enumerable: true,
  get() { return imageSource.get.call(this); },
  set(value) {
    imageSource.set.call(this, value);
    fetch(value)
      .then((response) => response.blob())
      .then((blob) => createImageBitmap(blob))
      .then((bitmap) => {
        this.data = bitmap.data;
        if (!this.hasAttribute('width')) this.width = bitmap.width;
        if (!this.hasAttribute('height')) this.height = bitmap.height;
        this.dispatchEvent(new Event('load'));
      })
      .catch((error) => {
        const event = new Event('error');
        event.error = error;
        this.dispatchEvent(event);
      });
  },
});

globalThis.Image = function Image(width, height) {
  const image = document.createElement('img');
  if (width !== undefined) image.width = width;
  if (height !== undefined) image.height = height;
  return image;
};
globalThis.Image.prototype = HTMLImageElement.prototype;

// Cooperative Web Worker execution. Worker source runs in its own lexical
// scope and communicates asynchronously; this preserves Worker messaging
// semantics for Draco/KTX2 decoders while the host runtime remains single-
// threaded and deterministic.
class Worker extends EventTarget {
  constructor(specifier, options = {}) {
    super();
    this.onmessage = null;
    this.onerror = null;
    this._terminated = false;
    this._workerHandler = null;
    this._ready = this._load(String(specifier), options);
  }

  async _load(specifier, options) {
    if (options.type === 'module') throw new TypeError('Module workers are not supported by the native asset worker');
    const url = new URL(specifier, globalThis.__exampleURL);
    let source;
    if (url.protocol === 'blob:') {
      const blob = globalThis.__blobFromObjectURL(url.href);
      if (blob === null) throw new TypeError(`Worker Blob URL does not exist: ${url.href}`);
      source = await blob.text();
    } else if (url.protocol === 'file:') {
      source = await (await fetch(url.href)).text();
    } else {
      throw new TypeError(`Worker does not support ${url.protocol} URLs`);
    }

    const workerSelf = Object.create(null);
    workerSelf.self = workerSelf;
    const workerListeners = new Map();
    workerSelf.addEventListener = (type, callback) => {
      const listeners = workerListeners.get(type) ?? [];
      listeners.push(callback);
      workerListeners.set(type, listeners);
    };
    workerSelf.removeEventListener = (type, callback) => {
      const listeners = workerListeners.get(type);
      if (listeners) workerListeners.set(type, listeners.filter((listener) => listener !== callback));
    };
    workerSelf.__dispatchEvent = (type, event) => {
      for (const listener of workerListeners.get(type) ?? []) listener.call(workerSelf, event);
    };
    for (const name of [
      'ArrayBuffer', 'SharedArrayBuffer', 'DataView',
      'Int8Array', 'Uint8Array', 'Uint8ClampedArray', 'Int16Array', 'Uint16Array',
      'Int32Array', 'Uint32Array', 'Float16Array', 'Float32Array', 'Float64Array', 'BigInt64Array', 'BigUint64Array',
    ]) {
      if (globalThis[name] !== undefined) workerSelf[name] = globalThis[name];
    }
    workerSelf.postMessage = (data, transfer = []) => {
      const cloned = structuredClone(data, { transfer });
      this._emitMessage(cloned);
    };
    workerSelf.close = () => { this._terminated = true; };
    const workerConsole = {
      ...console,
      error: (...values) => Deno.core.ops.op_probe_log(`worker console error: ${values.map((value) => value?.stack || String(value)).join(' ')}`),
    };
    workerSelf.console = workerConsole;
    workerSelf.WebAssembly = WebAssembly;
    workerSelf.TextDecoder = TextDecoder;
    workerSelf.TextEncoder = TextEncoder;
    this._workerScope = workerSelf;

    const bootstrap = new Function(
      'self', 'postMessage', 'close', 'console',
      `let onmessage;\n${source}\nreturn (event) => {\n` +
      `  const handler = onmessage ?? self.onmessage;\n` +
      `  if (typeof handler === 'function') handler.call(self, event);\n` +
      `  self.__dispatchEvent('message', event);\n` +
      `};`,
    );
    this._workerHandler = bootstrap(
      workerSelf,
      workerSelf.postMessage,
      workerSelf.close,
      workerConsole,
    );
  }

  postMessage(data, transfer = []) {
    if (this._terminated) return;
    const cloned = structuredClone(data, { transfer });
    this._ready
      .then(() => queueMicrotask(() => {
        if (this._terminated) return;
        if (typeof this._workerHandler !== 'function') throw new TypeError('Worker has no message handler');
        return this._workerHandler({ data: cloned });
      }))
      .catch((error) => this._emitError(error));
  }

  _emitMessage(data) {
    if (this._terminated) return;
    if (data?.type === 'error') {
      Deno.core.ops.op_probe_log(`worker error: ${data.error || data.message || JSON.stringify(data)}`);
    }
    queueMicrotask(() => {
      if (this._terminated) return;
      const event = new Event('message');
      Object.defineProperty(event, 'data', { value: data, enumerable: true });
      this.dispatchEvent(event);
      if (typeof this.onmessage === 'function') this.onmessage(event);
    });
  }

  _emitError(error) {
    if (this._terminated) return;
    const event = new Event('error');
    Object.defineProperty(event, 'error', { value: error, enumerable: true });
    this.dispatchEvent(event);
    if (typeof this.onerror === 'function') this.onerror(event);
    else queueMicrotask(() => { throw error; });
  }

  terminate() { this._terminated = true; }
}
globalThis.Worker = Worker;

// Timers are driven cooperatively by the host's __tick, but deadlines use the
// real monotonic clock saved before the deterministic harness freezes
// performance.now(). This preserves browser timer ordering and prevents a
// 100 ms interval from firing on every host tick during network-idle settling.
const __hostNow = performance.now.bind(performance);
let __deterministicTimerNow = null;
globalThis.__setDeterministicTimerTime = (milliseconds) => {
  __deterministicTimerNow = Number(milliseconds);
};
const timerNow = () => __deterministicTimerNow ?? __hostNow();
const __timers = new Map();
let __tid = 0;
const normalizeTimerDelay = (delay) => {
  const number = Number(delay);
  if (!Number.isFinite(number) || number < 0) return 0;
  return Math.min(number, 0x7fffffff);
};
const scheduleTimer = (callback, delay, repeat, args) => {
  if (typeof callback !== 'function') callback = Function(String(callback));
  const id = ++__tid;
  const interval = normalizeTimerDelay(delay);
  __timers.set(id, { callback, interval, repeat, args, deadline: timerNow() + interval });
  return id;
};
globalThis.setInterval = (callback, delay = 0, ...args) => scheduleTimer(callback, delay, true, args);
globalThis.clearInterval = (id) => { __timers.delete(Number(id)); };
globalThis.setTimeout = (callback, delay = 0, ...args) => scheduleTimer(callback, delay, false, args);
globalThis.clearTimeout = (id) => { __timers.delete(Number(id)); };
globalThis.__tick = () => {
  if (__timers.size === 0) return;
  const now = timerNow();
  const ids = Array.from(__timers.keys());
  let firstError;
  for (const id of ids) {
    const timer = __timers.get(id);
    if (!timer || timer.deadline > now) continue;
    if (timer.repeat) timer.deadline = now + timer.interval;
    else __timers.delete(id);
    try {
      timer.callback(...timer.args);
    } catch (error) {
      firstError ??= error;
    }
  }
  if (firstError) throw firstError;
};
globalThis.__timerCount = () => __timers.size;
// Production afterglow-shell uses deno_web's WHATWG timers, backed by
// core.createTimer and the host event-loop waker. The deterministic browser
// runner does not install this object and retains the manually ticked clock
// above for reproducible reference captures.
const __nativeTimers = globalThis.__afterglowTimersNative;
if (__nativeTimers) {
  globalThis.setTimeout = __nativeTimers.setTimeout;
  globalThis.clearTimeout = __nativeTimers.clearTimeout;
  globalThis.setInterval = __nativeTimers.setInterval;
  globalThis.clearInterval = __nativeTimers.clearInterval;
  globalThis.__tick = () => {};
  globalThis.__timerCount = () => 0;
}
globalThis.scheduler = globalThis.scheduler || {
  yield: () => new Promise((resolve) => setTimeout(resolve, 0)),
  postTask: (callback, _options = {}) => new Promise((resolve, reject) => {
    setTimeout(() => Promise.resolve().then(callback).then(resolve, reject), 0);
  }),
};

// Pointer Lock API: the native host grabs the cursor and reports raw
// DeviceEvent::MouseMotion as coalesced movementX/movementY on pointermove.
let __pointerLockElement = null;
const __dispatchPointerLockChange = () => {
  document.dispatchEvent(new Event('pointerlockchange'));
};
Object.defineProperty(Document.prototype, 'pointerLockElement', {
  configurable: true,
  enumerable: true,
  get() { return __pointerLockElement; },
});
HTMLElement.prototype.requestPointerLock = function (_options) {
  return new Promise((resolve, reject) => {
    try {
      Deno.core.ops.op_request_pointer_lock();
      __pointerLockElement = this;
      __dispatchPointerLockChange();
      resolve();
    } catch (error) {
      reject(error);
    }
  });
};
HTMLElement.prototype.exitPointerLock = function () {
  if (__pointerLockElement !== this) return;
  Deno.core.ops.op_exit_pointer_lock();
  __pointerLockElement = null;
  __dispatchPointerLockChange();
};
globalThis.__clearPointerLock = () => {
  if (__pointerLockElement !== null) {
    __pointerLockElement = null;
    __dispatchPointerLockChange();
  }
};

// WebGPU: hook canvas.getContext('webgpu') to return our capture canvas.
// (document.createElement('canvas') returns a LinkeDOM canvas; we provide the
// WebGPU context — the one environment piece LinkeDOM can't.)
const canvasContexts = new WeakMap();
const gpuCanvasIds = new WeakMap();
globalThis.__gpuCanvasId = (context) => gpuCanvasIds.get(context) ?? 0;
const canvasWidth = Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype, 'width');
const canvasHeight = Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype, 'height');
let nextCanvasId = 0;
globalThis.__gpuCanvasCount = 0;
const configureCanvas = (canvas) => {
  if (canvasContexts.has(canvas)) return canvas;
  const state = { id: ++nextCanvasId, type: null, gpu: null };
  canvasContexts.set(canvas, state);
  Object.defineProperties(canvas, {
    width: {
      configurable: true,
      get: () => canvasWidth.get.call(canvas),
      set: (value) => {
        canvasWidth.set.call(canvas, value);
        if (state.gpu) Deno.core.ops.op_resize_canvas(state.id, canvas.width, canvas.height);
      },
    },
    height: {
      configurable: true,
      get: () => canvasHeight.get.call(canvas),
      set: (value) => {
        canvasHeight.set.call(canvas, value);
        if (state.gpu) Deno.core.ops.op_resize_canvas(state.id, canvas.width, canvas.height);
      },
    },
  });
  canvas.getContext = (type) => {
    type = String(type).toLowerCase();
    if (type !== 'webgpu' && type !== '2d') return null;
    if (state.type !== null && state.type !== type) return null;
    state.type = type;
    if (type === '2d') return installCanvas2D(canvas);
    if (!state.gpu) {
      state.gpu = Deno.core.ops.op_create_capture_canvas(state.id, canvas.width || 300, canvas.height || 150);
      gpuCanvasIds.set(state.gpu, state.id);
      Deno.core.ops.op_bind_canvas_node(state.id, ensureNativeNodeId(canvas));
      globalThis.__gpuCanvasCount++;
    }
    return state.gpu;
  };
  return canvas;
};

for (const canvas of document.querySelectorAll('canvas')) configureCanvas(canvas);

const __origCreateElement = document.createElement.bind(document);
document.createElement = (tag) => {
  const element = __origCreateElement(tag);
  return String(tag).toLowerCase() === 'canvas' ? configureCanvas(element) : element;
};
