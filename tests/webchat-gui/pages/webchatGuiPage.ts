import { expect, type FrameLocator, type Locator, type Page } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';

export type Skin = 'default' | '3aigent';

export function tenantName(options: { skin: Skin; nav?: boolean; login?: boolean }): string {
  return `${options.skin}-${options.nav ? 'nav' : 'plain'}-${options.login ? 'login' : 'anon'}`;
}

export class WebChatGuiPage {
  readonly page: Page;

  constructor(page: Page) {
    this.page = page;
  }

  async installMockWebChat() {
    const mockPath = path.resolve(process.cwd(), 'tests/webchat-gui/fixtures/mock-webchat.js');
    const body = fs.readFileSync(mockPath, 'utf8');
    await this.page.route('https://cdn.botframework.com/botframework-webchat/latest/webchat.js', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'text/javascript; charset=utf-8',
        body,
      });
    });
  }

  fullscreenUrl(options: { skin: Skin; nav?: boolean; login?: boolean }) {
    const tenant = tenantName(options);
    return `/v1/web/webchat/${tenant}/?tenant=${tenant}`;
  }

  hostUrl(options: { skin: Skin; render: 'native' | 'iframe'; mode: 'inline' | 'launcher' | 'popup'; nav?: boolean; login?: boolean; adaptiveCardWidth?: string }) {
    const params = new URLSearchParams({
      skin: options.skin,
      render: options.render,
      mode: options.mode,
      nav: options.nav ? '1' : '0',
      login: options.login ? '1' : '0',
    });
    if (options.adaptiveCardWidth) {
      params.set('adaptiveCardWidth', options.adaptiveCardWidth);
    }
    return `/test-pages/host.html?${params.toString()}`;
  }

  async openFullscreen(options: { skin: Skin; nav?: boolean; login?: boolean }) {
    await this.page.goto(this.fullscreenUrl(options));
  }

  async openHost(options: { skin: Skin; render: 'native' | 'iframe'; mode: 'inline' | 'launcher' | 'popup'; nav?: boolean; login?: boolean; adaptiveCardWidth?: string }) {
    await this.page.goto(this.hostUrl(options));
    await expect(this.page.getByTestId('host-title')).toBeVisible();
  }

  chatInput(scope: Page | FrameLocator = this.page): Locator {
    return scope.getByTestId('webchat-input');
  }

  async expectChatReady(scope: Page | FrameLocator = this.page) {
    await expect(scope.getByTestId('webchat-root')).toBeVisible();
    await expect(this.chatInput(scope)).toBeVisible();
  }

  async sendMessage(text: string, scope: Page | FrameLocator = this.page) {
    await this.chatInput(scope).fill(text);
    await scope.getByTestId('webchat-send').click();
    await expect(scope.getByText(text, { exact: true })).toBeVisible();
    await expect(scope.getByText('Hello from Greentic').last()).toBeVisible();
  }

  async expectNoBrokenImages() {
    const broken = await this.page.evaluate(() => {
      return Array.from(document.images)
        .filter((image) => image.complete && image.naturalWidth === 0)
        .map((image) => image.getAttribute('src') || image.currentSrc);
    });
    expect(broken).toEqual([]);
  }

  async expectNoHorizontalOverflow() {
    const overflow = await this.page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(overflow).toBeLessThanOrEqual(2);
  }

  embeddedElement(): Locator {
    return this.page.locator('greentic-webchat').first();
  }

  iframeChat(): FrameLocator {
    return this.embeddedElement().frameLocator('iframe');
  }

  launcherButton(): Locator {
    return this.embeddedElement().locator('button.launcher');
  }
}
