/**
 * When the host hides its spinner.
 *
 * It used to hide on the iframe's own `load` event, which says the document
 * finished loading and nothing about whether the app has drawn anything. The
 * app then holds the screen with its own full-height "loading experience" card
 * while it resolves the tenant, so the host handed the user an apparently
 * blank panel for the whole of that wait. Measured against the live deployment
 * on a 4G profile: document load 1684 ms, `#root` first child 1698 ms (the
 * loading card), chat on screen 4094 ms.
 *
 * `runtime-bootstrap.js` now posts `greentic-webchat:ready` from inside the
 * frame once `#root` shows something that is not that card, and `embed.js`
 * hides the spinner on that message instead. Both files ship in the same pack,
 * so the two halves cannot reach a deployment at different versions.
 *
 * These tests slow the tenant config down on purpose. Served locally the whole
 * boot is a few dozen milliseconds, which is exactly the condition under which
 * the old behaviour looked correct.
 */
import { expect, test } from '@playwright/test';
import { WebChatGuiPage } from '../pages/webchatGuiPage';

/** Long enough that the old `load`-driven reveal would be unmistakably early,
 *  short enough to stay well inside the per-test timeout. */
const TENANT_CONFIG_DELAY_MS = 2_500;

/** The host's own safety net is 15 s; allow for it plus scheduling slack. */
const SAFETY_NET_TIMEOUT_MS = 25_000;

test.beforeEach(async ({ page }) => {
  await new WebChatGuiPage(page).installMockWebChat();
});

async function openLauncherWidget(page: import('@playwright/test').Page) {
  const webchat = new WebChatGuiPage(page);
  await webchat.openHost({ skin: 'default', render: 'iframe', mode: 'launcher', nav: false, login: false });
  await webchat.launcherButton().click();
  return webchat;
}

test.describe('the host spinner tracks the framed app, not the document', () => {
  test('it stays up after the iframe document has finished loading', async ({ page }) => {
    await page.route('**/config/tenants/*.json', async (route) => {
      await new Promise((resolve) => setTimeout(resolve, TENANT_CONFIG_DELAY_MS));
      await route.continue();
    });

    const webchat = await openLauncherWidget(page);
    const frame = webchat.iframeChat();

    // The moment the old code revealed the frame at. Everything the user would
    // have seen from here until `expectChatReady` below is the app's own
    // loading card -- or, through the host's opaque surface, nothing at all.
    await expect
      .poll(async () => frame.locator('#root').evaluate(() => document.readyState), { timeout: 15_000 })
      .toBe('complete');

    await expect(webchat.loadingOverlay()).toBeVisible();

    await webchat.expectChatReady(frame);
    await expect(webchat.loadingOverlay()).toBeHidden();
  });

  test('it clears on its own when the app never reports being ready', async ({ page }) => {
    // A config request that never answers: the frame loads, the app stays on
    // its loading card forever, and no ready message is ever posted. A spinner
    // that never clears is worse than one that clears too early, so the host
    // gives up by itself.
    await page.route('**/config/tenants/*.json', async () => {
      await new Promise(() => {});
    });

    const webchat = await openLauncherWidget(page);
    await expect(webchat.loadingOverlay()).toBeVisible();
    await expect(webchat.loadingOverlay()).toBeHidden({ timeout: SAFETY_NET_TIMEOUT_MS });
  });
});
