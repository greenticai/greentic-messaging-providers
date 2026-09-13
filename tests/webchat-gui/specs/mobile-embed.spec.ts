/**
 * The launcher-mode widget at phone size.
 *
 * `embedded.spec.ts` already opens the widget at 390x844, but it asserts only
 * `toBeInViewport()` and chat-readiness. Neither notices an element painted on
 * top of the panel, and neither notices that the panel cannot be dismissed --
 * which is how 0.5.30's fix for the launcher/composer overlap came to leave the
 * fullscreen chat with no way out. These assertions are about geometry and
 * about reachability of the close affordance.
 *
 * Two deliberate choices, both of which look like omissions:
 *
 * - **Not tagged `@mobile`.** That tag routes a test into the `mobile-chromium`
 *   project, whose Pixel 5 emulation cannot drive this host page at all: the
 *   launcher click times out with `host-content` intercepting pointer events,
 *   and it does so against an unmodified widget too. That is a harness gap, not
 *   a widget defect. These tests set their own viewport instead, which is what
 *   the existing mobile case in `embedded.spec.ts` does.
 * - **No `expectNoHorizontalOverflow()`.** `host.html`'s own header overflows a
 *   390px viewport by 25px before the widget is even opened, so that helper
 *   cannot say anything about the widget here. The dock's own box is measured
 *   instead.
 */
import { expect, test } from '@playwright/test';
import { WebChatGuiPage } from '../pages/webchatGuiPage';

const PHONE = { width: 390, height: 844 };
/** A phone held sideways: wide enough to stay docked, too short for the panel. */
const PHONE_LANDSCAPE = { width: 844, height: 390 };

test.beforeEach(async ({ page }) => {
  await new WebChatGuiPage(page).installMockWebChat();
});

async function openLauncherWidget(page: import('@playwright/test').Page) {
  const webchat = new WebChatGuiPage(page);
  await webchat.openHost({ skin: 'default', render: 'iframe', mode: 'launcher', nav: false, login: false });
  await webchat.launcherButton().click();
  await webchat.expectChatReady(webchat.iframeChat());
  return webchat;
}

test.describe('launcher widget at phone size', () => {
  test('the open fullscreen chat can still be closed', async ({ page }) => {
    await page.setViewportSize(PHONE);
    const webchat = await openLauncherWidget(page);

    // The launcher carries the close icon, so hiding it outright strands the
    // user: the only other close path is a `keydown` Escape handler, and a
    // phone has no Escape key.
    const launcher = webchat.launcherButton();
    await expect(launcher).toBeVisible();
    await launcher.click();
    await expect(webchat.embeddedElement()).toHaveJSProperty('open', false);
  });

  test('the launcher does not sit over the composer', async ({ page }) => {
    await page.setViewportSize(PHONE);
    const webchat = await openLauncherWidget(page);

    const launcher = await webchat.launcherButton().boundingBox();
    expect(launcher).not.toBeNull();
    // The composer is the bottom strip of a fullscreen panel. Anything floating
    // there lands on the send button -- which is the defect the launcher was
    // hidden for in the first place.
    expect(launcher!.y + launcher!.height).toBeLessThan(PHONE.height / 2);
  });

  test('the fullscreen panel fits the viewport', async ({ page }) => {
    await page.setViewportSize(PHONE);
    const webchat = await openLauncherWidget(page);

    const dock = await webchat.dock().boundingBox();
    expect(dock).not.toBeNull();
    // Safe-area padding is added OUTSIDE a declared height unless box-sizing
    // says otherwise, so an overflow here is the composer pushed off-screen.
    expect(dock!.y).toBeGreaterThanOrEqual(-1);
    expect(dock!.y + dock!.height).toBeLessThanOrEqual(PHONE.height + 1);
    expect(dock!.x).toBeGreaterThanOrEqual(-1);
    expect(dock!.x + dock!.width).toBeLessThanOrEqual(PHONE.width + 1);
  });

  test('the docked panel stays inside a short landscape viewport', async ({ page }) => {
    await page.setViewportSize(PHONE_LANDSCAPE);
    const webchat = await openLauncherWidget(page);

    const surface = await webchat.dockSurface().boundingBox();
    expect(surface).not.toBeNull();
    // Above the fullscreen breakpoint the panel is measured up from a bottom
    // offset, so a viewport shorter than the panel pushes its top off-screen --
    // the start of the transcript becomes unreachable.
    expect(surface!.y).toBeGreaterThanOrEqual(-1);
    expect(surface!.y + surface!.height).toBeLessThanOrEqual(PHONE_LANDSCAPE.height + 1);
  });

  test('desktop keeps the floating launcher below the open panel', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 960 });
    const webchat = await openLauncherWidget(page);

    // The desktop layout already clears the panel, so the mobile rules must not
    // reach it: the launcher stays at its bottom-right anchor.
    const launcher = await webchat.launcherButton().boundingBox();
    expect(launcher!.y).toBeGreaterThan(960 / 2);

    const surface = await webchat.dockSurface().boundingBox();
    expect(surface!.width).toBeLessThanOrEqual(421);
  });
});
