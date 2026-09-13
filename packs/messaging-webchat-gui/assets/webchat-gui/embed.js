const template = document.createElement("template");

// Static local custom-element template; no user-controlled input reaches this assignment.
// foxguard: ignore[js/no-xss-innerhtml]
template.innerHTML = `
  <style>
    :host {
      --greentic-webchat-z: 2147483646;
      --greentic-webchat-accent: #10b981;
      --greentic-webchat-accent-hover: #059669;
      --greentic-webchat-radius: 12px;
      display: block;
      width: 100%;
      height: 100%;
      min-height: 0;
      color-scheme: light;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }

    .frame {
      width: min(100%, 420px);
      height: min(680px, 80vh);
      border: 0;
      border-radius: var(--greentic-webchat-radius);
      box-shadow: 0 18px 50px rgba(15, 23, 42, 0.24);
      background: #fff;
      overflow: hidden;
    }

    .dock {
      position: fixed;
      right: 20px;
      bottom: 92px;
      z-index: var(--greentic-webchat-z);
      display: none;
    }

    .dock[data-open="true"] {
      display: block;
    }

    .inline[hidden],
    .dock[hidden],
    button.launcher[hidden] {
      display: none !important;
    }

    .inline {
      display: block;
      width: 100%;
      height: 100%;
      min-height: 0;
    }

    .inline .frame {
      width: 100%;
      height: 100%;
      min-height: 0;
      box-shadow: none;
    }

    .native {
      display: block;
      width: 100%;
      height: 100%;
      min-height: 0;
    }

    slot[name="native"] {
      display: block;
      width: 100%;
      height: 100%;
      min-height: 0;
    }

    ::slotted(.native) {
      display: block;
      width: 100%;
      height: 100%;
      min-height: 0;
    }

    button.launcher {
      position: fixed;
      right: 20px;
      bottom: 20px;
      z-index: var(--greentic-webchat-z);
      width: 56px;
      height: 56px;
      border: 0;
      border-radius: 50%;
      display: inline-grid;
      place-items: center;
      color: #fff;
      background: var(--greentic-webchat-accent);
      box-shadow: 0 12px 30px rgba(15, 23, 42, 0.28);
      cursor: pointer;
    }

    button.launcher:hover {
      background: var(--greentic-webchat-accent-hover);
    }

    button.launcher:focus-visible {
      outline: 3px solid rgba(16, 185, 129, 0.35);
      outline-offset: 3px;
    }

    .icon {
      width: 28px;
      height: 28px;
      fill: currentColor;
    }

    /* The dock's own close control. Hidden until the mobile fullscreen state
       below reveals it: on desktop the launcher sits clear of the panel and
       already doubles as the close affordance. Widget mode renders only the
       chat surface -- no app header -- so the top-right corner is free. */
    .dock .close {
      position: absolute;
      top: calc(8px + env(safe-area-inset-top, 0px));
      right: calc(8px + env(safe-area-inset-right, 0px));
      z-index: 1;
      display: none;
      place-items: center;
      width: 40px;
      height: 40px;
      border: 0;
      border-radius: 50%;
      color: #0f172a;
      background: rgba(255, 255, 255, 0.92);
      box-shadow: 0 2px 10px rgba(15, 23, 42, 0.22);
      cursor: pointer;
    }

    .dock .close:focus-visible {
      outline: 3px solid rgba(16, 185, 129, 0.35);
      outline-offset: 2px;
    }

    .close-icon {
      width: 20px;
      height: 20px;
      fill: currentColor;
    }

    /* Short viewport, still wide enough to stay docked: a phone in landscape,
       or a small desktop window. "min(680px, 80vh)" measured from a 92px
       bottom offset leaves roughly 230px of panel on a 390px-tall viewport,
       which is not a usable transcript. */
    @media (max-height: 520px) and (min-width: 521px) {
      .dock[data-render="iframe"] {
        bottom: 12px;
      }

      .dock[data-render="iframe"] .frame {
        height: calc(100vh - 24px);
        height: calc(100dvh - 24px);
      }
    }

    @media (max-width: 520px) {
      .dock[data-render="iframe"] {
        top: 0;
        right: 0;
        bottom: 0;
        left: 0;
        box-sizing: border-box;
        width: 100%;
        /* "100vh" on iOS Safari and Android Chrome measures the viewport with
           the URL bar RETRACTED, so the bottom 60-100px of the panel -- which
           is exactly where the composer lives -- sits below the fold until the
           user scrolls the host page. "dvh" tracks the bar as it moves; the
           "vh" line above it stays as the fallback for browsers without it.
           Both lines are needed -- a lone "dvh" is dropped silently. */
        height: 100vh;
        height: 100dvh;
        /* Keeps the composer clear of the iPhone home indicator. */
        padding-bottom: env(safe-area-inset-bottom, 0px);
        background: #fff;
      }

      .dock[data-render="iframe"] .frame {
        /* NOT "100vw", which counts the classic scrollbar gutter and overflows
           the host page horizontally. The dock is already edge-to-edge. */
        width: 100%;
        height: 100%;
        border-radius: 0;
        box-shadow: none;
      }

      /* The launcher is fixed-positioned at the SAME z-index as the dock and
         comes later in this template, so while the dock is fullscreen the
         launcher paints on top of it -- a 56px circle sitting directly over
         the send button. It is also the only way to close the chat, which is
         why it could not simply be dropped: ".close" above replaces it here. */
      .dock[data-render="iframe"][data-open="true"] ~ button.launcher {
        display: none !important;
      }

      .dock[data-render="iframe"][data-open="true"] .close {
        display: grid;
      }
    }
  </style>
  <div class="inline" part="inline" hidden></div>
  <slot name="native"></slot>
  <div class="dock" part="dock">
    <button class="close" part="close" type="button" aria-label="Close chat">
      <svg class="close-icon" viewBox="0 0 24 24" aria-hidden="true">
        <path d="M6.4 5 5 6.4 10.6 12 5 17.6 6.4 19 12 13.4 17.6 19 19 17.6 13.4 12 19 6.4 17.6 5 12 10.6 6.4 5Z"/>
      </svg>
    </button>
  </div>
  <button class="launcher" part="launcher" type="button" aria-expanded="false">
    <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 4h16v12H7.4L4 19.4V4Zm2 2v8.6l.6-.6H18V6H6Zm2 3h8v1.8H8V9Zm0 3h5v1.8H8V12Z"/>
    </svg>
  </button>
`;

function boolAttr(value, fallback = false) {
  if (value == null) return fallback;
  return value === "" || value === "true" || value === "1";
}

function normalizeAdaptiveCardWidth(value) {
  const raw = value == null ? "" : String(value).trim();
  if (!raw) return "70%";
  if (/^\d+(?:\.\d+)?$/.test(raw)) return `${raw}%`;
  if (/^\d+(?:\.\d+)?(?:%|px|rem|em|vw|vh)$/.test(raw)) return raw;
  if (raw.toLowerCase() === "auto") return "auto";
  return "70%";
}

function scriptPublicBaseUrl() {
  const current = import.meta.url || (document.currentScript && document.currentScript.src);
  if (!current) return window.location.origin;
  try {
    const url = new URL(current);
    const marker = "/v1/web/webchat/";
    const index = url.pathname.indexOf(marker);
    if (index >= 0) {
      return `${url.origin}${url.pathname.slice(0, index)}`;
    }
    return url.origin;
  } catch {
    return window.location.origin;
  }
}

const nativeAssetCache = new Map();
const nativeEmbedStyleId = "greentic-webchat-native-embed-style";

function appendOnce(selector, createElement) {
  const existing = document.head.querySelector(selector);
  if (existing) return existing;
  const element = createElement();
  document.head.append(element);
  return element;
}

function loadScriptOnce(src) {
  return new Promise((resolve, reject) => {
    const existing = document.head.querySelector(`script[src="${CSS.escape(src)}"]`);
    if (existing) {
      if (existing.dataset.loaded === "true") resolve();
      else existing.addEventListener("load", () => resolve(), { once: true });
      return;
    }
    const script = document.createElement("script");
    script.src = src;
    script.async = false;
    script.onload = () => {
      script.dataset.loaded = "true";
      resolve();
    };
    script.onerror = () => reject(new Error(`Failed to load ${src}`));
    document.head.append(script);
  });
}

function ensureNativeEmbedStyles() {
  appendOnce(`#${nativeEmbedStyleId}`, () => {
    const style = document.createElement("style");
    style.id = nativeEmbedStyleId;
    style.textContent = `
      .greentic-webchat-native-root,
      .greentic-webchat-native-root > * {
        width: 100%;
        height: 100%;
        min-height: 0;
      }

      .greentic-webchat-native-root .status-card,
      .greentic-webchat-native-root .app-shell,
      .greentic-webchat-native-root .login-shell {
        width: 100%;
        height: 100%;
        min-height: 0;
      }

      .greentic-webchat-native-root .embed-shell {
        width: 100%;
        height: 100%;
        min-height: 0;
      }

      .greentic-webchat-native-root .login-panel {
        min-height: 0;
        padding: 16px;
      }
    `;
    return style;
  });
}

async function discoverNativeAssets(appBaseUrl) {
  if (!nativeAssetCache.has(appBaseUrl)) {
    nativeAssetCache.set(
      appBaseUrl,
      // Native asset discovery fetches from the configured app base URL.
      // foxguard: ignore[js/no-ssrf]
      fetch(`${appBaseUrl}/index.html`, { cache: "no-store" })
        .then((response) => {
          if (!response.ok) throw new Error(`Failed to load ${appBaseUrl}/index.html`);
          return response.text();
        })
        .then((html) => {
          const doc = new DOMParser().parseFromString(html, "text/html");
          const moduleScript = doc.querySelector('script[type="module"][src]');
          const stylesheet = doc.querySelector('link[rel="stylesheet"][href]');
          if (!moduleScript) throw new Error("WebChat app module script not found");
          return {
            moduleUrl: new URL(moduleScript.getAttribute("src"), `${appBaseUrl}/`).toString(),
            cssUrl: stylesheet ? new URL(stylesheet.getAttribute("href"), `${appBaseUrl}/`).toString() : "",
            runtimeUrl: `${appBaseUrl}/runtime-bootstrap.js`,
          };
        })
    );
  }
  return nativeAssetCache.get(appBaseUrl);
}

async function loadNativeApp(appBaseUrl, tenant) {
  const assets = await discoverNativeAssets(appBaseUrl);
  ensureNativeEmbedStyles();
  if (assets.cssUrl) {
    appendOnce(`link[href="${CSS.escape(assets.cssUrl)}"]`, () => {
      const link = document.createElement("link");
      link.rel = "stylesheet";
      link.href = assets.cssUrl;
      return link;
    });
  }

  const appUrl = new URL(appBaseUrl, window.location.href);
  const appPath = appUrl.pathname.replace(/\/+$/, "");
  const appBasePath = `${appPath}/`;
  const configBaseUrl = `${appUrl.origin}${appBasePath}config`;

  document.documentElement.dataset.tenant = tenant;
  window.__TENANT__ = tenant;
  window.__BASE_PATH__ = appBasePath;
  window.APP_CONFIG_BASE = configBaseUrl;
  window.__WEBCHAT_GUI_BASE__ = appBasePath;
  window.__GREENTIC_WEBCHAT_FORCE_EMBED__ = true;
  await loadScriptOnce(assets.runtimeUrl);
  document.documentElement.dataset.tenant = tenant;
  window.__TENANT__ = tenant;
  window.__BASE_PATH__ = appBasePath;
  window.APP_CONFIG_BASE = configBaseUrl;
  window.__WEBCHAT_GUI_BASE__ = appBasePath;
  await import(assets.moduleUrl);
  if (!window.GreenticWebChatApp || typeof window.GreenticWebChatApp.mount !== "function") {
    throw new Error("WebChat app native mount API is unavailable");
  }
  return window.GreenticWebChatApp;
}

class GreenticWebchatElement extends HTMLElement {
  static get observedAttributes() {
    return [
      "tenant",
      "api-base",
      "public-base-url",
      "skin",
      "mode",
      "render",
      "launcher",
      "open",
      "locale",
      "text-input",
      "disable-text-input",
      "adaptive-card-width",
      "title",
      "close-title",
    ];
  }

  constructor() {
    super();
    this.attachShadow({ mode: "open" });
    this.shadowRoot.append(template.content.cloneNode(true));
    this._dock = this.shadowRoot.querySelector(".dock");
    this._inline = this.shadowRoot.querySelector(".inline");
    this._launcher = this.shadowRoot.querySelector(".launcher");
    this._close = this.shadowRoot.querySelector(".close");
    this._iframe = null;
    this._native = null;
    this._nativeMount = null;
    this._nativeToken = 0;
    this._iframeToken = 0;
    this._ready = false;
    this._launcher.addEventListener("click", () => this.toggle());
    this._close.addEventListener("click", () => this.close());
  }

  connectedCallback() {
    this.render();
    queueMicrotask(() => {
      if (!this._ready) {
        this._ready = true;
        this.dispatch("greentic-webchat-ready");
      }
    });
  }

  disconnectedCallback() {
    this._iframeToken++;
    this.unmountNative();
  }

  attributeChangedCallback() {
    if (this.isConnected) this.render();
  }

  get tenant() {
    return this.getAttribute("tenant") || "default";
  }

  set tenant(value) {
    this.setAttribute("tenant", value);
  }

  get open() {
    return this.hasAttribute("open");
  }

  set open(value) {
    if (value) this.setAttribute("open", "");
    else this.removeAttribute("open");
  }

  get launcher() {
    const mode = this.mode;
    if (mode === "inline" || mode === "popup") return false;
    if (mode === "launcher") return true;
    return boolAttr(this.getAttribute("launcher"), true);
  }

  get mode() {
    const value = (this.getAttribute("mode") || "").trim().toLowerCase();
    if (value === "inline" || value === "launcher" || value === "popup") return value;
    return this.hasAttribute("launcher") ? (boolAttr(this.getAttribute("launcher"), true) ? "launcher" : "inline") : "launcher";
  }

  get renderMode() {
    const value = (this.getAttribute("render") || "").trim().toLowerCase();
    return value === "native" ? "native" : "iframe";
  }

  get textInputEnabled() {
    if (boolAttr(this.getAttribute("disable-text-input"), false)) {
      return false;
    }
    return boolAttr(this.getAttribute("text-input"), true);
  }

  get adaptiveCardWidth() {
    return normalizeAdaptiveCardWidth(this.getAttribute("adaptive-card-width"));
  }

  set launcher(value) {
    if (value) this.setAttribute("launcher", "true");
    else this.setAttribute("launcher", "false");
  }

  show() {
    this.hidden = false;
  }

  hide() {
    this.hidden = true;
  }

  toggle() {
    this.open ? this.close() : this.openChat();
  }

  openChat() {
    if (!this.open) {
      this.open = true;
      this.dispatch("greentic-webchat-open");
    }
  }

  close() {
    if (this.open) {
      this.open = false;
      this.dispatch("greentic-webchat-close");
    }
  }

  dispatch(name, detail = {}) {
    this.dispatchEvent(new CustomEvent(name, { bubbles: true, composed: true, detail }));
  }

  render() {
    try {
      const useLauncher = this.launcher;
      const renderMode = this.renderMode;
      this._launcher.hidden = !useLauncher;
      this._launcher.setAttribute("aria-expanded", String(this.open));
      this._launcher.setAttribute("aria-label", this.getAttribute("title") || "Open chat");
      this._close.setAttribute("aria-label", this.getAttribute("close-title") || "Close chat");

      const target = useLauncher ? this._dock : this._inline;
      this._inline.hidden = useLauncher || renderMode === "native";
      this._dock.dataset.open = String(useLauncher && this.open);
      this._dock.dataset.render = renderMode;

      if (renderMode === "native") {
        this._iframeToken++;
        this._iframe && this._iframe.remove();
        this._iframe = null;
        this.mountNative();
        return;
      }

      this.unmountNative();

      if (!this._iframe || this._iframe.parentElement !== target) {
        this._iframe && this._iframe.remove();
        this._iframe = document.createElement("iframe");
        this._iframe.className = "frame";
        this._iframe.setAttribute("part", "iframe");
        this._iframe.setAttribute("allow", "clipboard-write");
        target.append(this._iframe);
      }

      this._iframe.title = this.getAttribute("title") || "Greentic WebChat";
      this._iframe.dataset.render = renderMode;
      this.scheduleIframeNavigation(this.webchatUrl());
    } catch (error) {
      this.dispatch("greentic-webchat-error", { message: String(error && error.message || error) });
    }
  }

  scheduleIframeNavigation(nextUrl) {
    if (!this._iframe) return;
    if (this._iframe.dataset.greenticSrc === nextUrl) return;
    const token = ++this._iframeToken;
    const iframe = this._iframe;
    const waitForLayout = (attempt = 0) => {
      if (token !== this._iframeToken || !this.isConnected || iframe !== this._iframe) return;
      const rect = iframe.getBoundingClientRect();
      const visible = rect.width > 120 && rect.height > 160 && iframe.offsetParent !== null;
      if (!visible && attempt < 30) {
        window.setTimeout(() => waitForLayout(attempt + 1), 50);
        return;
      }
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          if (token !== this._iframeToken || !this.isConnected || iframe !== this._iframe) return;
          iframe.dataset.greenticSrc = nextUrl;
          iframe.src = nextUrl;
        });
      });
    };
    waitForLayout();
  }

  mountNative() {
    if (!this._native) {
      this._native = document.createElement("div");
      this._native.className = "native greentic-webchat-native-root";
      this._native.slot = "native";
      this._native.setAttribute("part", "native");
      this._native.style.cssText = "display:block;width:100%;height:100%;min-height:0;";
      this.append(this._native);
    }
    const token = ++this._nativeToken;
    window.__GREENTIC_WEBCHAT_TEXT_INPUT_ENABLED__ = this.textInputEnabled;
    window.__GREENTIC_WEBCHAT_ADAPTIVE_CARD_WIDTH__ = this.adaptiveCardWidth;
    document.documentElement.style.setProperty("--greentic-adaptive-card-width", this.adaptiveCardWidth);
    loadNativeApp(this.appBaseUrl(), this.tenant)
      .then((app) => {
        if (token !== this._nativeToken || !this.isConnected || !this._native) return;
        if (!this._nativeMount) {
          this._nativeMount = app.mount(this._native);
        }
      })
      .catch((error) => {
        this.dispatch("greentic-webchat-error", { message: String(error && error.message || error) });
      });
  }

  unmountNative() {
    this._nativeToken++;
    if (this._nativeMount) {
      this._nativeMount.unmount();
      this._nativeMount = null;
    }
    if (this._native) {
      this._native.remove();
      this._native = null;
    }
  }

  webchatUrl() {
    const url = new URL(`${this.appBaseUrl()}/`);
    const apiBase = this.getAttribute("api-base");
    const skin = this.getAttribute("skin");
    const locale = this.getAttribute("locale");
    if (apiBase) url.searchParams.set("apiBase", apiBase);
    if (skin) url.searchParams.set("skin", skin);
    if (locale) url.searchParams.set("lang", locale);
    if (this.renderMode === "iframe") {
      url.searchParams.set("presentation_mode", "embed_webcomponent");
    }
    url.searchParams.set("adaptiveCardWidth", this.adaptiveCardWidth);
    if (!this.textInputEnabled) url.searchParams.set("textInput", "false");
    return url.toString();
  }

  appBaseUrl() {
    const publicBase = (this.getAttribute("public-base-url") || scriptPublicBaseUrl()).replace(/\/+$/, "");
    const tenant = encodeURIComponent(this.tenant);
    return `${publicBase}/v1/web/webchat/${tenant}`;
  }
}

if (!customElements.get("greentic-webchat")) {
  customElements.define("greentic-webchat", GreenticWebchatElement);
}

const legacyConfig = window.greenticChatConfig;
if (legacyConfig && !document.querySelector("greentic-webchat[data-greentic-legacy]")) {
  const element = document.createElement("greentic-webchat");
  element.dataset.greenticLegacy = "true";
  if (legacyConfig.tenant) element.setAttribute("tenant", legacyConfig.tenant);
  if (legacyConfig.baseUrl) element.setAttribute("public-base-url", legacyConfig.baseUrl);
  if (legacyConfig.apiBase) element.setAttribute("api-base", legacyConfig.apiBase);
  if (legacyConfig.skin) element.setAttribute("skin", legacyConfig.skin);
  if (legacyConfig.locale) element.setAttribute("locale", legacyConfig.locale);
  if (legacyConfig.textInput === false || legacyConfig.text_input_enabled === false) {
    element.setAttribute("text-input", "false");
  }
  if (legacyConfig.adaptiveCardWidth || legacyConfig.adaptive_card_width) {
    element.setAttribute("adaptive-card-width", legacyConfig.adaptiveCardWidth || legacyConfig.adaptive_card_width);
  }
  if (legacyConfig.openOnLoad) element.setAttribute("open", "");
  const appendLegacyElement = () => document.body.append(element);
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", appendLegacyElement, { once: true });
  } else {
    appendLegacyElement();
  }
  window.greenticChat = {
    open: () => element.openChat(),
    close: () => element.close(),
    toggle: () => element.toggle(),
    isOpen: () => element.open,
    hide: () => element.hide(),
    show: () => element.show(),
  };
}
