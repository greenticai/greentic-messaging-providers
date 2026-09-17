import { expect, test } from '@playwright/test';
import { WebChatGuiPage } from '../pages/webchatGuiPage';

// The browser's own language is deliberately English in every case below.
// `?lang=` must win over it: Web Chat stamps its locale onto every activity it
// sends, and the server honours that per-turn stamp over the conversation's
// language — so a page opened with `?lang=es` that hands Web Chat `en-US`
// renders the first card in Spanish and every card after it in English.
test.use({ locale: 'en-US' });

test.beforeEach(async ({ page }) => {
  await new WebChatGuiPage(page).installMockWebChat();
});

function recordedLocale(page: import('@playwright/test').Page) {
  return page.evaluate(() => (window as unknown as { __MOCK_WEBCHAT_LOCALE__?: string }).__MOCK_WEBCHAT_LOCALE__);
}

test.describe('locale selection', () => {
  test('?lang= reaches Web Chat instead of the browser language', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await page.goto('/v1/web/webchat/default/?tenant=default&lang=es');
    await webchat.expectChatReady();

    await expect.poll(() => recordedLocale(page)).toBe('es');
  });

  test('a later visit without ?lang= keeps the chosen language', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await page.goto('/v1/web/webchat/default/?tenant=default&lang=es');
    await webchat.expectChatReady();

    await page.goto('/v1/web/webchat/default/?tenant=default');
    await webchat.expectChatReady();

    await expect.poll(() => recordedLocale(page)).toBe('es');
    // The bootstrap's own half — the picker and the X-Greentic-Locale header on
    // conversation create — must agree with what Web Chat was handed.
    await expect
      .poll(() => page.evaluate(() => (window as unknown as { __SELECTED_LOCALE__?: string }).__SELECTED_LOCALE__))
      .toBe('es');
  });

  test('without ?lang= or a saved choice the browser language is used', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await page.goto('/v1/web/webchat/default/?tenant=default');
    await webchat.expectChatReady();

    await expect.poll(() => recordedLocale(page)).toMatch(/^en/);
  });
});
