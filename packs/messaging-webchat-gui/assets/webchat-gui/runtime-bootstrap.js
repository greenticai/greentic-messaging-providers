(function () {
  function injectAdaptiveCardLayoutOverrides() {
    if (document.getElementById('webchat-adaptive-card-layout-overrides')) {
      return;
    }

    const style = document.createElement('style');
    style.id = 'webchat-adaptive-card-layout-overrides';
    style.textContent = `
      #webchat .webchat__bubble,
      #webchat .webchat__bubble__content,
      #webchat .webchat__stacked-layout__message {
        max-width: 100%;
      }

      #webchat .ac-adaptiveCard,
      #webchat .ac-container,
      #webchat .ac-columnSet,
      #webchat .ac-textBlock {
        max-width: 100% !important;
      }

      #webchat .ac-adaptiveCard {
        width: 100% !important;
        box-sizing: border-box;
      }

      #webchat .ac-actionSet,
      #webchat .ac-horizontal-separator + div[role="group"] {
        display: flex !important;
        flex-wrap: wrap !important;
        gap: 12px;
        width: 100%;
      }

      #webchat .ac-actionSet > button,
      #webchat .ac-actionSet .ac-pushButton,
      #webchat .ac-horizontal-separator + div[role="group"] > button {
        flex: 1 1 220px;
        max-width: 100%;
        min-width: 0;
        white-space: normal !important;
        overflow-wrap: anywhere;
      }
    `;

    document.head.appendChild(style);
  }

  function resolveTenant() {
    const match = window.location.pathname.match(/\/v1\/web\/webchat\/([^\/?#]+)/i);
    if (match && match[1]) {
      return decodeURIComponent(match[1]);
    }
    const queryTenant = new URLSearchParams(window.location.search).get('tenant');
    if (queryTenant) {
      return queryTenant;
    }
    return document.documentElement?.dataset?.tenant || 'default';
  }

  function resolveGuiBase(tenant) {
    return `/v1/web/webchat/${encodeURIComponent(tenant)}/`;
  }

  function backendBase(tenant) {
    return `${window.location.origin}/v1/messaging/webchat/${encodeURIComponent(tenant)}`;
  }

  const tenant = resolveTenant();
  const guiBase = resolveGuiBase(tenant);

  injectAdaptiveCardLayoutOverrides();

  document.documentElement.dataset.tenant = tenant;
  window.__TENANT__ = tenant;
  window.__BASE_PATH__ = guiBase;
  window.APP_CONFIG_BASE = './config';
  window.__WEBCHAT_GUI_BASE__ = guiBase;
  window.__WEBCHAT_BACKEND_BASE__ = backendBase(tenant);

  const originalFetch = window.fetch.bind(window);
  window.fetch = function (input, init) {
    const requestUrl = typeof input === 'string' ? input : input.url;
    const url = new URL(requestUrl, window.location.href);

    if (/\/config\/tenants\/[^/]+\.json$/i.test(url.pathname)) {
      const tenantId = decodeURIComponent(url.pathname.split('/').pop().replace(/\.json$/i, ''));
      const payload = {
        tenant_id: tenantId,
        legacy_skin: '_template',
        branding: {
          company_name: tenantId
        },
        webchat: {
          directline: {
            token_url: `${window.location.origin}/v1/messaging/webchat/${encodeURIComponent(tenantId)}/token`,
            domain: `${window.location.origin}/v1/messaging/webchat/${encodeURIComponent(tenantId)}/v3/directline`
          },
          style_options: {
            bubbleMaxWidth: 1200
          },
          adaptive_cards_host_config: {
            actions: {
              actionsOrientation: 'horizontal',
              buttonSpacing: 12
            }
          },
          locale: 'en-US'
        },
        auth: {
          providers: [
            {
              id: `${tenantId}-demo`,
              label: 'Demo Login',
              type: 'dummy',
              enabled: true
            }
          ]
        }
      };
      return Promise.resolve(
        new Response(JSON.stringify(payload), {
          status: 200,
          headers: { 'Content-Type': 'application/json' }
        })
      );
    }

    return originalFetch(input, init);
  };
})();
