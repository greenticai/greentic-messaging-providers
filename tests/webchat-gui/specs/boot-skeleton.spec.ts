/**
 * The full-page boot screen.
 *
 * While the app resolves the tenant it renders one line of text on an empty
 * full-height card, and for the first moment of that the line is the raw key
 * `status.loadingExperience` -- its i18n catalog has not arrived either.
 * Measured against the live deployment on a 4G profile, that screen holds from
 * ~1.7s to ~4.1s.
 *
 * `runtime-bootstrap.js` covers it with a skeleton of the chat that is coming.
 * The overlay is a child of <body> and never of `#root`, which belongs to
 * React -- so these tests care most about it being REMOVED: a placeholder that
 * outlives the content it stood in for is the failure that mechanism exists to
 * avoid.
 *
 * The embedded widget has its own host-side spinner (`frame-ready-signal.spec.ts`);
 * this is the surface with no host to hide anything.
 */
import { expect, test } from '@playwright/test';
import { WebChatGuiPage } from '../pages/webchatGuiPage';

const SKELETON = '#greentic-webchat-boot-skeleton';

/** Long enough to outlast the 250 ms hold-back the overlay is mounted behind. */
const TENANT_CONFIG_DELAY_MS = 2_500;

test.beforeEach(async ({ page }) => {
  await new WebChatGuiPage(page).installMockWebChat();
});

test.describe('full-page boot skeleton', () => {
  test('it stands in for the loading card and is gone once the chat renders', async ({ page }) => {
    await page.route('**/config/tenants/*.json', async (route) => {
      await new Promise((resolve) => setTimeout(resolve, TENANT_CONFIG_DELAY_MS));
      await route.continue();
    });

    const webchat = new WebChatGuiPage(page);
    await webchat.openFullscreen({ skin: 'default', variant: 'skeleton' });

    await expect(page.locator(SKELETON)).toBeVisible();
    // The card's own text stays in the DOM behind the overlay so a screen
    // reader still hears "Loading tenant experience" rather than a description
    // of some bars.
    await expect(page.locator('.status-card')).toBeAttached();

    await webchat.expectChatReady();
    await expect(page.locator(SKELETON)).toHaveCount(0);
  });

  test('it goes up before the loading card can flash a raw i18n key', async ({ page }) => {
    // Nothing is slowed down here: this is the ordinary boot. The overlay is
    // mounted with no hold-back, because the thing it covers is ALREADY a
    // loader -- delaying would show the card's text, swap it for a skeleton,
    // then show the chat. What the text says for its first moments is
    // `status.loadingExperience` verbatim, which is the strongest reason not
    // to let it show at all.
    const webchat = new WebChatGuiPage(page);
    await webchat.openFullscreen({ skin: 'default', variant: 'skeleton-fast' });

    await expect(page.locator(SKELETON)).toBeVisible();
    await expect(page.getByText('status.loadingExperience')).toHaveCount(0);

    await webchat.expectChatReady();
    await expect(page.locator(SKELETON)).toHaveCount(0);
  });

  test('a native embed never mounts one into the customer page', async ({ page }) => {
    // A native embed loads this same bundle into the customer's own page,
    // where there is no `#root` -- so nothing there could ever satisfy the
    // "app has painted" test and the overlay would cover their site
    // permanently. It first showed up as embedded.spec.ts timing out on a
    // click the overlay was intercepting; assert the cause, not the symptom.
    const webchat = new WebChatGuiPage(page);
    await webchat.openHost({ skin: 'default', render: 'native', mode: 'inline', nav: false, login: false });
    await webchat.expectChatReady();
    await expect(page.locator(SKELETON)).toHaveCount(0);
  });
});
