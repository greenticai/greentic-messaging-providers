/**
 * The launcher-mode widget at phone size.
 *
 * `embedded.spec.ts` already opens the widget at 390x844, but it asserts only
 * `toBeInViewport()` and chat-readiness — neither of which notices an element
 * painted ON TOP of the panel, which is how the launcher came to sit over the
 * send button on every phone without a single test going red. These assertions
 * are about geometry and stacking rather than reachability.
 *
 * Two deliberate choices, both of which look like omissions:
 *
 * - **Not tagged `@mobile`.** That tag routes a test into the `mobile-chromium`
 *   project, whose Pixel 5 emulation cannot drive this host page at all: the
 *   launcher click times out with `host-content` intercepting pointer events,
 *   and it does so against the UNPATCHED widget too. That is a harness gap, not
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
  test('the launcher does not cover the fullscreen panel', async ({ page }) => {
    await page.setViewportSize(PHONE);
    const webchat = await openLauncherWidget(page);

    // The launcher and the dock carry the SAME z-index and the launcher comes
    // later in the template, so anything still painted here lands on top of the
    // chat — in practice a 56px circle directly over the send button.
    await expect(webchat.launcherButton()).toBeHidden();
  });

  test('the fullscreen panel exposes a close control that works', async ({ page }) => {
    await page.setViewportSize(PHONE);
    const webchat = await openLauncherWidget(page);

    // Hiding the launcher removes the only way out, so the dock has to carry
    // its own: these assertions belong together.
    const close = webchat.closeButton();
    await expect(close).toBeVisible();
    await close.click();

    await expect(webchat.embeddedElement()).toHaveJSProperty('open', false);
    await expect(webchat.launcherButton()).toBeVisible();
  });

  test('the fullscreen panel fits the viewport', async ({ page }) => {
    await page.setViewportSize(PHONE);
    const webchat = await openLauncherWidget(page);

    const frame = await webchat.dockFrame().boundingBox();
    expect(frame).not.toBeNull();
    // The composer sits at the bottom of the panel, so an overhang here is the
    // input control falling below the fold.
    expect(frame!.y).toBeGreaterThanOrEqual(-1);
    expect(frame!.y + frame!.height).toBeLessThanOrEqual(PHONE.height + 1);
    expect(frame!.x).toBeGreaterThanOrEqual(-1);
    expect(frame!.x + frame!.width).toBeLessThanOrEqual(PHONE.width + 1);
  });

  test('the docked panel stays inside a short landscape viewport', async ({ page }) => {
    await page.setViewportSize(PHONE_LANDSCAPE);
    const webchat = await openLauncherWidget(page);

    const frame = await webchat.dockFrame().boundingBox();
    expect(frame).not.toBeNull();
    // Above the fullscreen breakpoint the panel is measured from a bottom
    // offset, so a viewport shorter than the panel pushes its top off-screen —
    // the start of the transcript becomes unreachable.
    expect(frame!.y).toBeGreaterThanOrEqual(-1);
    expect(frame!.y + frame!.height).toBeLessThanOrEqual(PHONE_LANDSCAPE.height + 1);
  });

  test('the native renderer is left alone at phone size', async ({ page }) => {
    await page.setViewportSize(PHONE);
    const webchat = new WebChatGuiPage(page);
    await webchat.openHost({ skin: 'default', render: 'native', mode: 'launcher', nav: true, login: false });
    await webchat.launcherButton().click();
    await webchat.expectChatReady();

    // `render()` stamps `data-open` on the dock for BOTH renderers, but the
    // native one slots its chat into the element's own box and leaves the dock
    // empty. An ungated fullscreen rule therefore paints an opaque white dock
    // over the host page, with the chat behind it and the launcher hidden.
    await expect(page.getByTestId('host-content')).toBeVisible();
    await expect(webchat.closeButton()).toBeHidden();
    const dock = await webchat.dock().boundingBox();
    expect(dock === null || dock.width < PHONE.width).toBeTruthy();
  });

  test('desktop keeps the floating launcher beside the open panel', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 960 });
    const webchat = await openLauncherWidget(page);

    // The desktop layout already clears the panel, so the mobile rules must not
    // reach it: the launcher stays, and the dock's close control stays hidden.
    await expect(webchat.launcherButton()).toBeVisible();
    await expect(webchat.closeButton()).toBeHidden();

    const frame = await webchat.dockFrame().boundingBox();
    expect(frame!.width).toBeLessThanOrEqual(421);
  });
});
