#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC_DIR="${GREENTIC_WEBCHAT_SITE_DIR:-/projects/ai/greentic-ng/greentic-webchat/site/app}"
DEST_DIR="${ROOT_DIR}/packs/messaging-webchat-gui/assets/webchat-gui"

if [ ! -d "${SRC_DIR}" ]; then
  echo "Skipping WebChat GUI asset import; source not found: ${SRC_DIR}" >&2
  exit 0
fi

mkdir -p "${DEST_DIR}" "${DEST_DIR}/config" "${DEST_DIR}/i18n" "${DEST_DIR}/js" "${DEST_DIR}/skins"

rsync -a --delete "${SRC_DIR}/assets/" "${DEST_DIR}/assets/"
rsync -a --delete "${SRC_DIR}/config/" "${DEST_DIR}/config/"
rsync -a --delete "${SRC_DIR}/i18n/" "${DEST_DIR}/i18n/"
rsync -a --delete "${SRC_DIR}/js/" "${DEST_DIR}/js/"
rsync -a --delete "${SRC_DIR}/skins/" "${DEST_DIR}/skins/"

js_bundle="$(basename "$(find "${DEST_DIR}/assets" -maxdepth 1 -type f -name 'index-*.js' | sort | head -n 1)")"
css_bundle="$(basename "$(find "${DEST_DIR}/assets" -maxdepth 1 -type f -name 'index-*.css' | sort | head -n 1)")"

if [ -z "${js_bundle}" ] || [ -z "${css_bundle}" ]; then
  echo "Unable to locate greentic-webchat app bundles in ${DEST_DIR}/assets" >&2
  exit 1
fi

cat > "${DEST_DIR}/runtime-bootstrap.js" <<'EOF'
(function () {
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
EOF

cat > "${DEST_DIR}/index.html" <<EOF
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Greentic WebChat</title>
    <script src="./runtime-bootstrap.js"></script>
    <script type="module" crossorigin src="./assets/${js_bundle}"></script>
    <link rel="stylesheet" crossorigin href="./assets/${css_bundle}">
  </head>
  <body>
    <div id="root"></div>
  </body>
</html>
EOF

cp "${DEST_DIR}/index.html" "${DEST_DIR}/404.html"
