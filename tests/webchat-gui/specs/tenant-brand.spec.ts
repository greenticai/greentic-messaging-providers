import { expect, test, type Page } from '@playwright/test';
import { WebChatGuiPage } from '../pages/webchatGuiPage';

/**
 * Operator-set tenant brand.
 *
 * greentic-setup copies the pack's `brand_name` / `brand_logo_url` answers into
 * `brand: { name, logo_url }` in the tenant config, and runtime-bootstrap.js
 * lays them over the skin's own brand. The skin's static shell hardcodes the
 * Greentic name and mark, so without that overlay every tenant's page reads
 * "Greentic" however the deployment was branded.
 *
 * The fixture names the scenario after the tenant: any tenant containing
 * `brand` carries the Meridian brand, `brand-http` a logo the page must refuse.
 */

const LOGO_URL = 'https://brand.example.test/logo.svg';
const LOGO_SVG =
  '<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" rx="12" fill="#1d4ed8"/></svg>';

async function serveBrandLogo(page: Page) {
  await page.route(LOGO_URL, (route) =>
    route.fulfill({ status: 200, contentType: 'image/svg+xml', body: LOGO_SVG }),
  );
}

test.describe('tenant brand', () => {
  test('the tenant name and logo replace the Greentic brand on the default skin', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await webchat.installMockWebChat();
    await serveBrandLogo(page);

    await webchat.openFullscreen({ skin: 'default', variant: 'brand' });
    await webchat.expectChatReady();

    await expect(page.locator('.topbar__title')).toHaveText('Meridian Insurance');
    await expect(page.locator('.footer__brand')).toHaveText('Meridian Insurance');
    await expect(page).toHaveTitle('Meridian Insurance');

    const logo = page.locator('.topbar__logo');
    await expect(logo).toHaveAttribute('src', LOGO_URL);
    await expect(logo).toHaveAttribute('alt', 'Meridian Insurance logo');
    await expect(logo).toHaveClass(/topbar__logo--tenant/);
    await expect(page.locator('.topbar__avatar')).toHaveClass(/topbar__avatar--tenant/);

    const skinBrand = await page.evaluate(
      () => (window as unknown as { __SKIN__?: { brand?: { name?: string; logo?: string } } }).__SKIN__?.brand ?? null,
    );
    expect(skinBrand).toEqual(expect.objectContaining({ name: 'Meridian Insurance', logo: LOGO_URL }));

    await webchat.expectNoBrokenImages();
  });

  test('the tenant brand also replaces the 3aigent mark', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await webchat.installMockWebChat();
    await serveBrandLogo(page);

    await webchat.openFullscreen({ skin: '3aigent', variant: 'brand' });
    await webchat.expectChatReady();

    await expect(page.locator('.topbar__title')).toHaveText('Meridian Insurance');
    await expect(page.locator('img.topbar__brand')).toHaveAttribute('src', LOGO_URL);
    await expect(page.locator('.footer__brand')).toHaveText('Meridian Insurance');
  });

  test('a tenant with no brand keeps the skin brand', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await webchat.installMockWebChat();

    await webchat.openFullscreen({ skin: 'default', variant: 'skin-only' });
    await webchat.expectChatReady();

    await expect(page.locator('.topbar__title')).toHaveText('Greentic');
    await expect(page.locator('.footer__brand')).toHaveText('Greentic');
    const logo = page.locator('.topbar__logo');
    await expect(logo).toHaveAttribute('src', /\/skins\/default\/assets\/logo\.svg$/);
    await expect(logo).not.toHaveClass(/topbar__logo--tenant/);
  });

  test('a non-https logo is refused while the name still applies', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await webchat.installMockWebChat();
    const warnings: string[] = [];
    page.on('console', (message) => {
      if (message.type() === 'warning') warnings.push(message.text());
    });

    await webchat.openFullscreen({ skin: 'default', variant: 'brand-http' });
    await webchat.expectChatReady();

    await expect(page.locator('.topbar__title')).toHaveText('Meridian Insurance');
    const logo = page.locator('.topbar__logo');
    await expect(logo).toHaveAttribute('src', /\/skins\/default\/assets\/logo\.svg$/);
    await expect(logo).not.toHaveClass(/topbar__logo--tenant/);
    expect(warnings.some((text) => text.includes('logo_url ignored'))).toBe(true);
  });
});
