import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, '../../..');
const assetRoot = path.join(repoRoot, 'packs/messaging-webchat-gui/assets/webchat-gui');
const testPagesRoot = path.join(repoRoot, 'tests/webchat-gui/test-pages');

const args = new Map();
for (let index = 2; index < process.argv.length; index += 1) {
  if (process.argv[index].startsWith('--')) {
    args.set(process.argv[index].slice(2), process.argv[index + 1]);
    index += 1;
  }
}
const port = Number(args.get('port') || process.env.WEBCHAT_GUI_TEST_PORT || 8799);

// Keyed per tenant (every test uses a distinct tenant name) so concurrent
// /token calls from unrelated tests running in parallel can't clobber each
// other's recorded header.
const lastTokenAuthorizationByTenant = new Map();

const demoLinks = [
  { id: 'docs', label: 'Docs', url: 'https://docs.greentic.ai' },
  { id: 'playground', label: 'Playground', url: 'https://example.test/playground' },
];

function contentType(filePath) {
  const ext = path.extname(filePath).toLowerCase();
  return {
    '.css': 'text/css; charset=utf-8',
    '.html': 'text/html; charset=utf-8',
    '.ico': 'image/x-icon',
    '.jpg': 'image/jpeg',
    '.jpeg': 'image/jpeg',
    '.js': 'text/javascript; charset=utf-8',
    '.json': 'application/json; charset=utf-8',
    '.map': 'application/json; charset=utf-8',
    '.png': 'image/png',
    '.svg': 'image/svg+xml',
  }[ext] || 'application/octet-stream';
}

function safeJoin(root, requestPath) {
  const decoded = decodeURIComponent(requestPath);
  const relative = decoded.replace(/^\/+/, '');
  const resolved = path.resolve(root, relative);
  if (!resolved.startsWith(root)) return null;
  return resolved;
}

function tenantScenario(tenant) {
  const normalized = String(tenant || 'default');
  const skin = normalized.includes('3aigent') ? '3aigent' : 'default';
  return {
    tenant: normalized,
    skin,
    nav: normalized.includes('nav'),
    login: normalized.includes('login'),
    tenantConfigLogin: normalized.includes('tenant-config-login'),
    ssoLogin: normalized.includes('sso'),
    // The shipped pack scaffold carries an enabled provider the operator never
    // asked for, while /auth/config -- which DOES honour `oauth_enabled` --
    // answers `{enabled:false}`. Every other scenario here either deletes the
    // static auth block or takes /auth/config away, so this disagreement was
    // the one shape nothing modelled.
    staleAuthProvider: normalized.includes('stale-auth'),
    // What greentic-setup writes from the pack's brand_name / brand_logo_url
    // answers. `brand-http` carries a logo the page must refuse to render.
    brand: normalized.includes('brand'),
    brandInsecureLogo: normalized.includes('brand-http'),
  };
}

function tenantConfig(tenant) {
  const scenario = tenantScenario(tenant);
  const basePath = path.join(assetRoot, 'config/tenants/greentic.json');
  const base = JSON.parse(fs.readFileSync(basePath, 'utf8'));
  base.tenant_id = scenario.tenant;
  // gtc-faithful: greentic-setup scaffolds <tenant>.json from default.json
  // (legacy_skin = "default") and sync_skin writes ONLY the `skin` field —
  // it never updates legacy_skin. Mirroring that here keeps the suite honest:
  // setting legacy_skin = scenario.skin previously masked the skin-selection
  // bug where the SPA picks the skin folder from legacy_skin.
  base.legacy_skin = 'default';
  base.skin = scenario.skin;
  base.branding = {
    company_name: scenario.skin === '3aigent' ? '3AIgent' : 'Greentic',
    tagline: scenario.skin === '3aigent' ? 'Deep Research' : 'Greentic AI Assistant',
    logo: scenario.skin === '3aigent'
      ? '/skins/3aigent/assets/3point-jewel-round.png'
      : '/skins/default/assets/logo.svg',
  };
  if (scenario.brand) {
    base.brand = {
      name: 'Meridian Insurance',
      logo_url: scenario.brandInsecureLogo
        ? 'http://brand.example.test/logo.svg'
        : 'https://brand.example.test/logo.svg',
    };
  }
  base.navigation = scenario.nav ? { menu: demoLinks } : { menu: [] };
  base.nav_links = scenario.nav ? demoLinks : [];
  delete base.auth;
  if (scenario.tenantConfigLogin) {
    base.auth = {
      providers: [
        { id: 'guest', label: 'Continue as Guest', type: 'dummy', enabled: true },
      ],
    };
  }
  if (scenario.staleAuthProvider) {
    // Verbatim from messaging-webchat-gui 0.5.17's own
    // config/tenants/default.json.
    base.auth = {
      providers: [
        {
          id: 'greentic',
          label: 'Greentic SSO',
          type: 'greentic',
          enabled: true,
          clientId: 'webchat-gui',
          scope: 'openid profile email greentic.webchat',
        },
      ],
    };
  }
  return base;
}

function send(res, status, headers, body) {
  res.writeHead(status, {
    'Cache-Control': 'no-store',
    'Access-Control-Allow-Origin': '*',
    'Access-Control-Allow-Headers': 'authorization,content-type,x-greentic-locale',
    'Access-Control-Allow-Methods': 'GET,POST,OPTIONS',
    ...headers,
  });
  res.end(body);
}

function sendJson(res, status, value) {
  send(res, status, { 'Content-Type': 'application/json; charset=utf-8' }, JSON.stringify(value));
}

function serveFile(res, filePath) {
  fs.stat(filePath, (statError, stat) => {
    if (statError || !stat.isFile()) {
      sendJson(res, 404, { error: 'not found' });
      return;
    }
    send(res, 200, { 'Content-Type': contentType(filePath) }, fs.readFileSync(filePath));
  });
}

function appAssetPath(urlPath) {
  const match = urlPath.match(/^\/v1\/web\/webchat\/[^/]+\/?(.*)$/);
  if (!match) return null;
  const rest = match[1] || 'index.html';
  const direct = safeJoin(assetRoot, rest || 'index.html');
  if (direct && fs.existsSync(direct)) return direct;
  // gtc addresses one deployment of a tenant as
  // /v1/web/webchat/{tenant}/{bundle_id}[/{flow_id}], and every asset the page
  // pulls is relative to that, so the bundle segment prefixes them all. The
  // pack's own runtime-bootstrap parses those segments off the page URL; the
  // fixture served only the bundle-less shape, so nothing here could exercise
  // a bundle-scoped deployment -- which is the shape gtc actually ships.
  const segments = (rest || '').split('/').filter(Boolean);
  if (segments.length === 0) return safeJoin(assetRoot, 'index.html');
  const withoutBundle = segments.slice(1).join('/');
  return safeJoin(assetRoot, withoutBundle || 'index.html');
}

// The Greentic Designer proxies a running operator environment at
// /api/operator-env/<id>/proxy/<path> as a byte passthrough: it rewrites no
// HTML, injects no <base href>, and sets no prefix header. Modelling that here
// is what lets the suite serve the GUI from somewhere other than the site root
// — the only condition under which a root-rooted runtime URL is wrong.
//
// The prefix is stripped and then discarded, exactly like the real conduit:
// nothing downstream learns about it, so the page's own URL stays the only
// evidence it exists. Tests assert on the request URLs the page emits, not on
// whether this server happened to answer them.
const MOUNT_PREFIX_PATTERN = /^\/api\/operator-env\/[^/]+\/proxy(?=\/)/;

const server = http.createServer((req, res) => {
  const url = new URL(req.url || '/', `http://${req.headers.host || '127.0.0.1'}`);
  const urlPath = url.pathname.replace(MOUNT_PREFIX_PATTERN, '');

  if (req.method === 'OPTIONS') {
    send(res, 204, {}, '');
    return;
  }
  if (urlPath === '/healthz') {
    sendJson(res, 200, { ok: true });
    return;
  }
  if (urlPath === '/favicon.ico') {
    serveFile(res, path.join(assetRoot, 'skins/default/assets/favicon.ico'));
    return;
  }
  if (urlPath === '/') {
    send(res, 302, { Location: '/host.html' }, '');
    return;
  }
  if (urlPath === '/mock-api/messages' && req.method === 'POST') {
    req.resume();
    sendJson(res, 200, { text: 'Hello from Greentic' });
    return;
  }
  if (urlPath.endsWith('/auth/config')) {
    const tenant = urlPath.split('/v1/messaging/webchat/')[1]?.split('/')[0] || 'default';
    const scenario = tenantScenario(tenant);
    if (scenario.tenantConfigLogin) {
      sendJson(res, 404, { error: 'auth config unavailable for tenant-config fallback scenario' });
      return;
    }
    if (scenario.ssoLogin) {
      sendJson(res, 200, {
        enabled: true,
        providers: [{
          id: 'greentic',
          label: 'Greentic SSO',
          type: 'greentic',
          enabled: true,
          issuer: `http://127.0.0.1:${port}/mock-idp`,
          client_id: 'webchat-gui',
        }],
      });
      return;
    }
    sendJson(res, 200, scenario.login
      ? { enabled: true, providers: [{ id: 'test-login', label: 'Test Login', type: 'dummy', enabled: true }] }
      : { enabled: false });
    return;
  }
  if (urlPath === '/mock-api/last-token-authorization') {
    const tenant = url.searchParams.get('tenant') || 'default';
    sendJson(res, 200, { authorization: lastTokenAuthorizationByTenant.get(tenant) || null });
    return;
  }
  if (urlPath.endsWith('/token') || urlPath.endsWith('/v3/directline/tokens/generate')) {
    req.resume();
    const tenant = urlPath.split('/v1/messaging/webchat/')[1]?.split('/')[0] || 'default';
    lastTokenAuthorizationByTenant.set(tenant, req.headers['authorization'] || null);
    sendJson(res, 200, {
      conversationId: 'webchat-gui-test',
      token: 'webchat-gui-test-token',
      expires_in: 1800,
    });
    return;
  }
  if (urlPath.includes('/v3/directline/conversations')) {
    req.resume();
    sendJson(res, 200, { activities: [], watermark: '0', id: 'mock-activity' });
    return;
  }
  // Every route below is reachable both bare and under a {bundle_id}, because
  // gtc addresses one deployment of a tenant as /v1/web/webchat/{tenant}/{bundle_id}
  // and the SPA resolves all of its own URLs relative to that. The optional group
  // backtracks out of the way on the bare shape, so both spell the same route.
  const tenantConfigMatch = urlPath.match(
    /^\/v1\/web\/webchat\/([^/]+)(?:\/([^/]+))?\/config\/tenants\/([^/]+)\.json$/,
  );
  if (tenantConfigMatch) {
    if (tenantConfigMatch[3].includes('missing-config')) {
      serveFile(res, path.join(assetRoot, 'index.html'));
      return;
    }
    sendJson(res, 200, tenantConfig(tenantConfigMatch[3]));
    return;
  }
  if (/^\/v1\/web\/webchat\/[^/]+(?:\/[^/]+)?\/i18n\/_manifest\.json$/.test(urlPath)) {
    sendJson(res, 200, { locales: ['en'] });
    return;
  }
  if (/^\/v1\/web\/webchat\/[^/]+(?:\/[^/]+)?\/i18n\/en-US\.json$/.test(urlPath)) {
    serveFile(res, path.join(assetRoot, 'i18n/en.json'));
    return;
  }
  // Real gtc exposes a single static route — /v1/web/webchat/{tenant} — and does
  // NOT serve /skins/ at the web root. Skin assets must resolve under the tenant
  // route (handled by appAssetPath below). This root /skins/ alias is opt-in via
  // WEBCHAT_LEGACY_SKINS_ALIAS=1 so the suite tests the gtc routing reality by
  // default; serving it unconditionally masked custom-skin asset 404s.
  if (process.env.WEBCHAT_LEGACY_SKINS_ALIAS === '1' && urlPath.startsWith('/skins/')) {
    const filePath = safeJoin(assetRoot, urlPath);
    if (filePath) serveFile(res, filePath);
    else sendJson(res, 403, { error: 'forbidden' });
    return;
  }
  if (urlPath.startsWith('/test-pages/')) {
    const filePath = safeJoin(testPagesRoot, urlPath.replace(/^\/test-pages\//, ''));
    if (filePath) serveFile(res, filePath);
    else sendJson(res, 403, { error: 'forbidden' });
    return;
  }
  const appPath = appAssetPath(urlPath);
  if (appPath) {
    serveFile(res, fs.existsSync(appPath) && fs.statSync(appPath).isDirectory() ? path.join(appPath, 'index.html') : appPath);
    return;
  }
  if (urlPath.endsWith('.map')) {
    send(res, 204, { 'Content-Type': 'application/json; charset=utf-8' }, '');
    return;
  }
  sendJson(res, 404, { error: 'not found', path: urlPath });
});

server.listen(port, '127.0.0.1', () => {
  console.log(`WebChat GUI Playwright server listening on http://127.0.0.1:${port}`);
});
