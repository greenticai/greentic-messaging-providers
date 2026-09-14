import { expect, test, type Page } from '@playwright/test';
import { WebChatGuiPage, type Skin } from '../pages/webchatGuiPage';

const skins: Skin[] = ['default', '3aigent'];

test.beforeEach(async ({ page }) => {
  const webchat = new WebChatGuiPage(page);
  await webchat.installMockWebChat();
  const consoleErrors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text());
  });
  page.on('response', (response) => {
    if (response.status() >= 400) consoleErrors.push(`${response.status()} ${response.url()}`);
  });
  page.on('pageerror', (error) => consoleErrors.push(error.message));
  await page.exposeFunction('__webchatConsoleErrors', () => consoleErrors);
});

async function expectNoConsoleErrors(page: Page) {
  const errors = await page.evaluate(async () => {
    return await (window as unknown as { __webchatConsoleErrors: () => Promise<string[]> }).__webchatConsoleErrors();
  });
  expect(errors.filter((line) => !line.includes('favicon'))).toEqual([]);
}

test.describe('full-screen WebChat', () => {
  for (const skin of skins) {
    test(`${skin} skin loads anonymously with chat input`, async ({ page }) => {
      const webchat = new WebChatGuiPage(page);
      await webchat.openFullscreen({ skin, nav: false, login: false });

      await expect(page.locator('.topbar__title')).toContainText(skin === '3aigent' ? '3AIgent' : 'Greentic');
      await webchat.expectChatReady();
      await webchat.sendMessage('Hello');
      await webchat.expectNoBrokenImages();
      await webchat.expectNoHorizontalOverflow();
      await expectNoConsoleErrors(page);
    });
  }

  test('navigation links are visible when configured', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await webchat.openFullscreen({ skin: 'default', nav: true, login: false });

    await expect(page.getByRole('navigation', { name: 'Site navigation' }).getByRole('link', { name: 'Docs' })).toBeVisible();
    await expect(page.getByRole('navigation', { name: 'Site navigation' }).getByRole('link', { name: 'Playground' })).toBeVisible();
    await webchat.expectChatReady();
  });

  test('navigation links are hidden when disabled', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await webchat.openFullscreen({ skin: 'default', nav: false, login: false });

    await expect(page.getByRole('navigation', { name: 'Site navigation' }).getByRole('link', { name: 'Docs' })).toHaveCount(0);
    await webchat.expectChatReady();
  });

  test('configured login gate appears and transitions to chat', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await webchat.openFullscreen({ skin: 'default', nav: false, login: true });

    await expect(page.locator('[data-i18n-key="login.title"]')).toBeVisible();
    const loginButton = page.getByRole('button', { name: /test login/i });
    await expect(loginButton).toBeVisible();
    await loginButton.click();
    await webchat.expectChatReady();
    await expect(page.evaluate(() => sessionStorage.getItem('greentic_oauth_token_handle'))).resolves.toBe('guest');
  });

  test('tenant-config login fallback appears when auth config endpoint is unavailable', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await page.goto('/v1/web/webchat/tenant-config-login/?tenant=tenant-config-login&loginRequired=1');

    await expect(page.getByRole('heading', { name: /sign in to greentic/i })).toBeVisible();
    await page.getByRole('button', { name: /continue as guest/i }).click();
    await webchat.expectChatReady();
    await expect(page.evaluate(() => {
      const raw = localStorage.getItem('webchat_auth_session');
      return raw ? JSON.parse(raw).isAuthenticated === true : false;
    })).resolves.toBe(true);
  });

  /**
   * `/auth/config` is the only surface that reads the operator's own
   * `oauth_enabled` answer. The static tenant JSON is a pack scaffold, and
   * messaging-webchat-gui 0.5.17 shipped it carrying an ENABLED Greentic SSO
   * provider nobody asked for.
   *
   * The SPA gates on `providers.filter(p => p.enabled).length > 0` and never
   * consults `/auth/config` at all, so that scaffold locked every visitor of
   * every deployment built from that pack behind a sign-in page -- while the
   * answers said `oauth_enabled: false` and `/auth/config` agreed, and no
   * layer reported a thing.
   *
   * The backend answer wins. It is what the operator actually configured.
   */
  test('a disabled auth config beats an enabled provider left in the tenant scaffold', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await webchat.installMockWebChat();
    await page.goto('/v1/web/webchat/stale-auth/acme-customer-service/');

    // Chat first: it is the assertion that WAITS. Asserting the absence of the
    // login chrome ahead of it passes on an empty page and would go green
    // against the very bug this pins.
    await webchat.expectChatReady();
    await expect(page.locator('[data-i18n-key="login.title"]')).toHaveCount(0);
    await expect(page.getByRole('button', { name: /greentic sso/i })).toHaveCount(0);
  });

  test('anonymous mode skips the login page', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await webchat.openFullscreen({ skin: 'default', nav: false, login: false });

    await expect(page.locator('[data-i18n-key="login.title"]')).toHaveCount(0);
    await webchat.expectChatReady();
  });

  test('adaptive cards default to 70 percent width', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await webchat.openFullscreen({ skin: 'default', nav: false, login: false });
    await webchat.expectChatReady();

    await expect(page.getByTestId('adaptive-card')).toBeVisible();
    await expect.poll(async () => page.getByTestId('adaptive-card').evaluate((card) => {
      const bubble = card.closest('.webchat__bubble__content');
      const transcript = card.closest('[data-testid="webchat-transcript"]');
      if (!bubble || !transcript) return 0;
      return bubble.getBoundingClientRect().width / transcript.getBoundingClientRect().width;
    })).toBeCloseTo(0.7, 1);
  });

  test('3aigent light mode keeps adaptive card text readable', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await page.addInitScript(() => sessionStorage.setItem('greentic-theme', 'light'));
    await webchat.openFullscreen({ skin: '3aigent', nav: false, login: false });
    await webchat.expectChatReady();

    const color = await page.getByTestId('adaptive-card-title').evaluate((element) => {
      return window.getComputedStyle(element).color;
    });
    const channels = color.match(/\d+(\.\d+)?/g)?.slice(0, 3).map(Number) ?? [];
    expect(channels).toHaveLength(3);
    expect(Math.max(...channels)).toBeLessThan(100);
  });

  test('default skin uses Greentic green adaptive card action styling', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await webchat.openFullscreen({ skin: 'default', nav: false, login: false });
    await webchat.expectChatReady();

    const action = page.getByTestId('adaptive-card-action');
    await expect(action).toBeVisible();
    const styles = await action.evaluate((element) => {
      const computed = window.getComputedStyle(element);
      // Resolve the skin's own `--brand` through the same engine, so this
      // asserts the RELATIONSHIP the test name claims — the action follows the
      // brand — rather than one particular green. A literal here is what let
      // `page.css` drift to #059669 while `skin.json` and `styleOptions.json`
      // both said #0b7f5b: the literal pinned the outlier, so the disagreement
      // was green in CI for as long as it existed.
      const probe = document.createElement('span');
      probe.style.color = 'var(--brand)';
      document.documentElement.append(probe);
      const brand = window.getComputedStyle(probe).color;
      probe.remove();
      return {
        backgroundColor: computed.backgroundColor,
        borderColor: computed.borderTopColor,
        color: computed.color,
        brand,
      };
    });
    expect(styles.backgroundColor).toBe('rgba(0, 0, 0, 0)');
    expect(styles.brand).not.toBe('');
    expect(styles.borderColor).toBe(styles.brand);
    expect(styles.color).toBe(styles.brand);
  });

  for (const skin of skins) {
    test(`${skin} skin differentiates adaptive card action styles`, async ({ page }) => {
      const webchat = new WebChatGuiPage(page);
      await webchat.openFullscreen({ skin, nav: false, login: false });
      await webchat.expectChatReady();

      const defaultAction = page.getByTestId('adaptive-card-action');
      const positiveAction = page.getByTestId('adaptive-card-action-positive');
      const destructiveAction = page.getByTestId('adaptive-card-action-destructive');
      await expect(defaultAction).toBeVisible();
      await expect(positiveAction).toBeVisible();
      await expect(destructiveAction).toBeVisible();

      const styles = await Promise.all(
        [defaultAction, positiveAction, destructiveAction].map((locator) => locator.evaluate((element) => {
          const computed = window.getComputedStyle(element);
          return {
            backgroundColor: computed.backgroundColor,
            borderColor: computed.borderTopColor,
            color: computed.color,
          };
        }))
      );
      const [defaultStyles, positiveStyles, destructiveStyles] = styles;

      expect(defaultStyles.backgroundColor).toBe('rgba(0, 0, 0, 0)');
      expect(positiveStyles.backgroundColor).not.toBe(defaultStyles.backgroundColor);
      expect(positiveStyles.color).toBe('rgb(255, 255, 255)');
      expect(destructiveStyles.backgroundColor).toBe('rgb(220, 38, 38)');
      expect(destructiveStyles.borderColor).toBe('rgb(252, 165, 165)');
      expect(destructiveStyles.color).toBe('rgb(255, 255, 255)');
    });
  }

  test('default dark mode keeps adaptive card action borders green and visible', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await page.addInitScript(() => sessionStorage.setItem('greentic-theme', 'dark'));
    await webchat.openFullscreen({ skin: 'default', nav: false, login: false });
    await webchat.expectChatReady();

    await expect(page.getByTestId('adaptive-card-action')).toBeVisible();
    const borderColor = await page.getByTestId('adaptive-card-action').evaluate((element) => {
      return window.getComputedStyle(element).borderTopColor;
    });
    expect(borderColor).toBe('rgb(52, 211, 153)');
  });

  for (const skin of skins) {
    test(`${skin} skin uses brand color for adaptive card links`, async ({ page }) => {
      const webchat = new WebChatGuiPage(page);
      await webchat.openFullscreen({ skin, nav: false, login: false });
      await webchat.expectChatReady();

      const link = page.getByTestId('adaptive-card-link');
      const email = page.getByTestId('adaptive-card-email');
      await expect(link).toHaveAttribute('href', 'https://adaptivecards.io/');
      await expect(email).toHaveAttribute('href', 'mailto:support@greentic.ai');

      await expect.poll(async () => link.evaluate((node, activeSkin) => {
        const rootStyle = getComputedStyle(document.documentElement);
        const theme = document.documentElement.getAttribute('data-theme');
        const expectedVariable = activeSkin === '3aigent' && theme !== 'light'
          ? '--brand-light'
          : activeSkin === '3aigent'
            ? '--brand-dark'
            : '--brand';
        const probe = document.createElement('span');
        probe.style.color = rootStyle.getPropertyValue(expectedVariable).trim();
        document.body.appendChild(probe);
        const expected = getComputedStyle(probe).color;
        probe.remove();
        return getComputedStyle(node).color === expected;
      }, skin)).toBe(true);
    });

    test(`${skin} skin shows adaptive card action spinner while pending`, async ({ page }) => {
      const webchat = new WebChatGuiPage(page);
      await webchat.openFullscreen({ skin, nav: false, login: false });
      await webchat.expectChatReady();

      const action = page.getByTestId('adaptive-card-action');
      await expect(action).toBeVisible();
      await action.click();

      await expect(action).toBeDisabled();
      await expect(action).toHaveAttribute('aria-busy', 'true');
      await expect.poll(async () => action.evaluate((button) => {
        const before = getComputedStyle(button, '::before');
        return {
          animationName: before.animationName,
          content: before.content,
          cursor: getComputedStyle(button).cursor,
        };
      })).toMatchObject({
        animationName: 'ac-action-spin',
        content: '""',
        cursor: 'wait',
      });
    });
  }

  test('missing tenant config does not invent a login page', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    const tokenRequests: string[] = [];
    page.on('request', request => {
      const url = request.url();
      if (url.includes('/v1/messaging/webchat/') && url.includes('/token')) {
        tokenRequests.push(url);
      }
    });
    await page.goto('/v1/web/webchat/missing-config-anon/?tenant=missing-config-anon');

    await expect(page.locator('[data-i18n-key="login.title"]')).toHaveCount(0);
    await expect(page.getByRole('button', { name: /sign in/i })).toHaveCount(0);
    await webchat.expectChatReady();
    expect(tokenRequests.some(url => url.includes('/v1/messaging/webchat/missing-config-anon/token'))).toBe(true);
    expect(tokenRequests.some(url => url.includes('/v1/messaging/webchat/greentic/token'))).toBe(false);
  });

  test('login callback error shows a clear failure state', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await webchat.openFullscreen({ skin: 'default', nav: false, login: true });
    await page.goto(`${webchat.fullscreenUrl({ skin: 'default', nav: false, login: true })}&error=access_denied&error_description=Denied`);

    await expect(page.getByRole('heading', { name: 'Something went wrong' })).toBeVisible();
    await expect(page.getByText(/Authentication failed: Denied/)).toBeVisible();
  });

  test('mobile viewport keeps chat input usable @mobile', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    const webchat = new WebChatGuiPage(page);
    await webchat.openFullscreen({ skin: '3aigent', nav: true, login: false });

    await webchat.expectChatReady();
    await expect(webchat.chatInput()).toBeInViewport();
    await webchat.expectNoHorizontalOverflow();
  });

  test('sso callback page loads the SDK and exposes the completer', async ({ page }) => {
    await page.goto('/v1/web/webchat/default-plain-anon/sso-callback.html');
    const hasCompleter = await page.evaluate(
      () => typeof (window as any).GreenticSso?.completeCallbackFromPopup === 'function',
    );
    expect(hasCompleter).toBe(true);
  });

  // Stubs GreenticSso.createGreenticWebchatSso before runtime-bootstrap.js reads
  // it. The bundle exports it as a non-configurable getter, so it can't be
  // reassigned in place — instead a property-descriptor trap on window.GreenticSso
  // swaps in a Proxy the instant the bundle assigns the real module object,
  // forwarding everything except the one factory method.
  function installGreenticSsoStub(stubMode: 'pending' | 'popup_blocked' | 'sub-only') {
    const wrap = (sdk: any) =>
      new Proxy(sdk, {
        get(target, prop, receiver) {
          if (prop === 'createGreenticWebchatSso') {
            return () => ({
              login: () => {
                (window as any).__SSO_LOGIN_CALLED__ = true;
                // A successful greentic login reloads once, so the React SPA
                // re-reads webchat_auth_session. Record the call somewhere that
                // survives that reload, or the assertion races the navigation.
                try { sessionStorage.setItem('__SSO_LOGIN_CALLED__', '1'); } catch { /* ignore */ }
                if (stubMode === 'popup_blocked') {
                  return Promise.reject(Object.assign(new Error('popup blocked'), { code: 'popup_blocked' }));
                }
                if (stubMode === 'sub-only') {
                  // Identity carrying only the SDK-guaranteed `sub` field —
                  // an IdP that omits name/email must still get a distinct cache key.
                  return Promise.resolve({ sub: 'sso-user-42' });
                }
                return new Promise(() => {});
              },
              isAuthenticated: () => false,
            });
          }
          return Reflect.get(target, prop, receiver);
        },
      });
    const existing = (window as any).GreenticSso;
    if (existing) {
      (window as any).GreenticSso = wrap(existing);
      return;
    }
    Object.defineProperty(window, 'GreenticSso', {
      configurable: true,
      set(value) {
        delete (window as any).GreenticSso;
        (window as any).GreenticSso = wrap(value);
      },
      get() {
        return undefined;
      },
    });
  }

  test('greentic sso provider renders first and drives the SDK', async ({ page }) => {
    await page.addInitScript(installGreenticSsoStub, 'pending');
    await page.goto('/v1/web/webchat/default-plain-sso/');
    const overlay = page.locator('#greentic-oauth-overlay');
    await expect(overlay).toBeVisible();
    const firstButton = overlay.locator('button').first();
    await expect(firstButton).toHaveText(/greentic sso/i);

    const built = await page.evaluate(() => {
      const cfg = (window as any).__OAUTH_CONFIG__;
      return cfg && cfg.providers && cfg.providers[0] && cfg.providers[0].type;
    });
    expect(built).toBe('greentic');

    await firstButton.click();
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as any).__SSO_LOGIN_CALLED__ === true ||
            sessionStorage.getItem('__SSO_LOGIN_CALLED__') === '1',
        ),
      )
      .toBe(true);
    const clientLive = await page.evaluate(() => !!(window as any).__GREENTIC_SSO_CLIENT__);
    expect(clientLive).toBe(true);
  });

  test('popup_blocked falls back to the redirect flow', async ({ page }) => {
    await page.addInitScript(installGreenticSsoStub, 'popup_blocked');
    const navigations: string[] = [];
    page.on('framenavigated', (f) => navigations.push(f.url()));
    await page.goto('/v1/web/webchat/default-plain-sso/');
    await page.locator('#greentic-oauth-overlay button').first().click();
    await expect
      .poll(() => navigations.some((u) => u.includes('/mock-idp/oauth/authorize')))
      .toBe(true);
  });

  test('SSO identity with only a sub scopes the Direct Line cache to that sub, not the shared token_handle', async ({ page }) => {
    await page.addInitScript(installGreenticSsoStub, 'sub-only');
    await page.goto('/v1/web/webchat/default-plain-sso/');
    await page.locator('#greentic-oauth-overlay button').first().click();
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as any).__SSO_LOGIN_CALLED__ === true ||
            sessionStorage.getItem('__SSO_LOGIN_CALLED__') === '1',
        ),
      )
      .toBe(true);

    // Every SSO identity here shares the same token_handle ('greentic-sso');
    // only `sub` distinguishes users. Poll for the sub-scoped key specifically
    // rather than "any" dl:token key, since an anonymous prefetch key can
    // also be present.
    await expect.poll(async () =>
      page.evaluate(() => Object.keys(localStorage).some((k) => k.includes(':dl:token:') && k.endsWith(':sso-user-42'))),
    ).toBe(true);

    const hasSharedTokenHandleKey = await page.evaluate(() =>
      Object.keys(localStorage).some((k) => k.includes(':dl:token:') && k.endsWith(':greentic-sso')),
    );
    expect(hasSharedTokenHandleKey).toBe(false);
  });

  test('logout clears the cached Direct Line token', async ({ page }) => {
    await page.goto('/v1/web/webchat/default-plain-login/');
    await page.getByRole('button', { name: /test login/i }).click();

    // The widget prefetches a Direct Line token on every page load regardless
    // of auth state, so an anonymous-scoped token key can appear (and later
    // reappear post-logout) independent of login. Poll for the identity-scoped
    // key specifically — the dummy 'test login' provider always sets user_name
    // to 'Guest', so the key's trailing segment is ':guest' — rather than "any"
    // dl:token key, so this doesn't race the anonymous prefetch's own key.
    await expect.poll(async () =>
      page.evaluate(() => Object.keys(localStorage).some((k) => k.includes(':dl:token:') && k.endsWith(':guest'))),
    ).toBe(true);

    const loggedInTokenKey = await page.evaluate(
      () => Object.keys(localStorage).find((k) => k.includes(':dl:token:') && k.endsWith(':guest')) as string,
    );
    expect(loggedInTokenKey).toBeTruthy();

    await page.evaluate(() => {
      const btn = document.getElementById('greentic-logout-btn') as HTMLButtonElement | null;
      btn?.click();
    });

    // Logout triggers window.location.reload(); page.evaluate() can hit an
    // in-flight navigation and reject, so treat that as "not settled yet"
    // instead of failing the poll outright.
    await expect.poll(async () => {
      try {
        return await page.evaluate((key) => localStorage.getItem(key), loggedInTokenKey);
      } catch (_) {
        return 'pending';
      }
    }).toBeNull();
  });

  test('login overlay uses i18n keys rather than hardcoded English', async ({ page }) => {
    await page.goto('/v1/web/webchat/default-plain-login/');
    const usesKeys = await page.evaluate(() => {
      const card = document.querySelector('#greentic-oauth-overlay h2');
      return card ? card.getAttribute('data-i18n-key') : null;
    });
    expect(usesKeys).toBe('login.title');
  });

  test('login overlay interpolates i18n placeholders instead of leaking literal braces', async ({ page }) => {
    await page.goto('/v1/web/webchat/default-plain-login/');
    const titleText = await page.locator('[data-i18n-key="login.title"]').textContent();
    expect(titleText).not.toContain('{{');
    const buttonText = await page.getByRole('button', { name: /test login/i }).textContent();
    expect(buttonText).not.toContain('{{');
    expect(buttonText).toContain('Test Login');
  });

  test('an SSO session attaches a bearer to the token mint', async ({ page, request }) => {
    await page.addInitScript(() => {
      (window as any).__GREENTIC_SSO_CLIENT__ = {
        getAccessToken: () => Promise.resolve('fake-access-token'),
      };
    });
    await page.goto('/v1/web/webchat/default-plain-anon/');
    await expect
      .poll(async () => {
        const res = await request.get('/mock-api/last-token-authorization?tenant=default-plain-anon');
        return (await res.json()).authorization;
      })
      .toBe('Bearer fake-access-token');
  });

  test('a dead SSO session shows the login screen instead of silently going anonymous', async ({ page }) => {
    const tokenAuthHeaders: string[] = [];
    page.on('request', (req) => {
      let pathname = '';
      try { pathname = new URL(req.url()).pathname; } catch (_) { /* ignore */ }
      if (/\/token$/.test(pathname)) tokenAuthHeaders.push(req.headers()['authorization'] || '');
    });
    // Seed once, not on every reload — the fix reloads the page, and
    // addInitScript re-runs on that reload too. Re-seeding an "existing
    // session" on the reloaded page would loop forever instead of settling
    // on the login screen, which is not how a real cleared session behaves.
    await page.addInitScript(() => {
      if (localStorage.getItem('__test_seeded_dead_sso_session__')) return;
      localStorage.setItem('__test_seeded_dead_sso_session__', '1');
      sessionStorage.setItem('greentic_oauth_token_handle', 'expired-access-token');
      sessionStorage.setItem('greentic_oauth_flow_id', 'greentic');
      sessionStorage.setItem('greentic_oauth_provider', JSON.stringify({ id: 'greentic', type: 'greentic' }));
      (window as any).__GREENTIC_SSO_CLIENT__ = {
        getAccessToken: () => Promise.reject(new Error('refresh failed')),
      };
    });
    await page.goto('/v1/web/webchat/default-plain-sso/');

    await expect(page.locator('[data-i18n-key="login.title"]')).toBeVisible();
    // The dead session's bearer must never reach the network — not even as
    // part of a request that otherwise succeeds anonymously.
    expect(tokenAuthHeaders.some((h) => h.includes('expired-access-token'))).toBe(false);
  });
});
