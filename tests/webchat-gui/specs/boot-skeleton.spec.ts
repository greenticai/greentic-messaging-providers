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
 * React -- so these tests care most about what it must NOT do: outlive the
 * content it stood in for, appear on a page that is not the app's own, or
 * disturb the app's boot.
 *
 * The embedded widget has its own host-side spinner (`frame-ready-signal.spec.ts`);
 * this is the surface with no host to hide anything.
 */
import { expect, test } from '@playwright/test';
import { WebChatGuiPage } from '../pages/webchatGuiPage';

const SKELETON = '#greentic-webchat-boot-skeleton';

/** Long enough to hold the loading card open across the whole assertion. */
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

  test('the chat still boots when the machine is slow', async ({ page, browserName }) => {
    test.skip(browserName !== 'chromium', 'CPU throttling is a CDP-only control');

    // The SPA gates its Web Chat init on `document.readyState === "complete"`,
    // seeded into React state at first render and otherwise only ever set by a
    // `window.load` listener attached in an effect -- so a first commit that
    // lands AFTER `load` attaches that listener too late and Web Chat never
    // mounts. That race is in the bundle and not editable from this repo.
    //
    // Mounting the overlay at DOMContentLoaded put enough work on the main
    // thread to push React's first commit into exactly that window: chat went
    // from ready in ~1.2s to never ready, and 15 CI tests failed while every
    // one of them passed on an unthrottled laptop. It is mounted at `load`
    // instead, which cannot reach the race.
    //
    // Throttling is what makes this reproducible off a busy CI runner. Without
    // it the regression is invisible locally, which is how it shipped.
    const session = await page.context().newCDPSession(page);
    await session.send('Emulation.setCPUThrottlingRate', { rate: 6 });

    const webchat = new WebChatGuiPage(page);
    await webchat.openFullscreen({ skin: 'default', variant: 'skeleton-slow' });
    await webchat.expectChatReady();
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
