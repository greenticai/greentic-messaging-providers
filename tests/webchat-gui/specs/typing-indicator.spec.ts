import fs from 'node:fs';
import path from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { WebChatGuiPage, type Skin } from '../pages/webchatGuiPage';

/**
 * Typing indicator.
 *
 * Web Chat mounts an empty `.webchat__typing-indicator` (a white 64x20 GIF box)
 * while the bot is typing. runtime-bootstrap.js draws one of three looks inside
 * it instead, chosen by the tenant config's `typing_indicator` (the pack's
 * setup answer): shimmer (default), pulse or dots.
 *
 * The mock Web Chat renders no typing indicator, so each test mounts a
 * Web-Chat-shaped one next to a bot avatar and bubble carrying the skin's
 * palette, which is what the indicator samples its colours from.
 *
 * Set TYPING_SHOTS_DIR to also write a screenshot of every case.
 */

type Mode = 'shimmer' | 'pulse' | 'dots';
type Theme = 'light' | 'dark';

const PALETTE: Record<Skin, Record<Theme, { bg: string; bubble: string; text: string; border: string; avatar: string }>> = {
  default: {
    light: { bg: '#d2dbd6', bubble: '#ffffff', text: '#101613', border: '#b3c2b9', avatar: '#0b7f5b' },
    dark: { bg: '#111827', bubble: '#1f2937', text: '#e5e7eb', border: '#334155', avatar: '#0b7f5b' },
  },
  '3aigent': {
    light: { bg: '#ffffff', bubble: '#f1f5f9', text: '#334155', border: 'transparent', avatar: '#0891b2' },
    dark: { bg: '#171717', bubble: '#1a1a1a', text: '#e5e7eb', border: 'transparent', avatar: '#06b6d4' },
  },
};

async function mountTypingScene(page: Page, skin: Skin, theme: Theme) {
  const p = PALETTE[skin][theme];
  await page.evaluate(
    ({ theme, p }) => {
      document.documentElement.setAttribute('data-theme', theme);
      const scene = document.createElement('div');
      scene.id = 'typing-scene';
      scene.style.cssText = `position:fixed;left:24px;bottom:24px;z-index:99999;width:420px;padding:14px;border-radius:10px;background:${p.bg};font-family:Inter,system-ui,sans-serif;`;
      scene.innerHTML = `
        <div style="display:flex;gap:8px;align-items:flex-start;margin-bottom:12px">
          <div class="webchat__initialsAvatar" style="flex:0 0 28px;height:28px;border-radius:50%;display:flex;align-items:center;justify-content:center;font-size:11px;background:${p.avatar};color:#fff">AI</div>
          <div class="webchat__bubble"><div class="webchat__bubble__content" style="background:${p.bubble};color:${p.text};border:1px solid ${p.border};border-radius:16px;padding:7px 12px;font-size:14px">What would you like help with today?</div></div>
        </div>
        <div class="typing-outer" style="padding:0 0 10px 0">
          <div aria-hidden="true" class="webchat__typing-indicator" style="background-color:#fff;width:64px;height:20px"></div>
        </div>`;
      document.body.appendChild(scene);
    },
    { theme, p },
  );
  const indicator = page.locator('#typing-scene .webchat__typing-indicator .gt-typing');
  await expect(indicator).toHaveCount(1);
  return indicator;
}

async function shoot(page: Page, name: string) {
  const dir = process.env.TYPING_SHOTS_DIR;
  if (!dir) return;
  fs.mkdirSync(dir, { recursive: true });
  await page.locator('#typing-scene').screenshot({ path: path.join(dir, `${name}.png`), animations: 'allow' });
}

async function open(page: Page, skin: Skin, variant: string) {
  const webchat = new WebChatGuiPage(page);
  await webchat.installMockWebChat();
  await webchat.openFullscreen({ skin, variant });
  await webchat.expectChatReady();
}

test.describe('typing indicator', () => {
  for (const mode of ['shimmer', 'pulse', 'dots'] as Mode[]) {
    for (const theme of ['light', 'dark'] as Theme[]) {
      test(`${mode} replaces the stock GIF box (${theme})`, async ({ page }) => {
        await open(page, 'default', `typing-${mode}-${theme}`);
        await expect(page.locator('html')).toHaveAttribute('data-typing-indicator', mode);

        const indicator = await mountTypingScene(page, 'default', theme);
        await expect(indicator).toHaveAttribute('data-mode', mode);
        await expect(indicator.locator('.gt-typing__avatar')).toHaveText('AI');

        const stock = page.locator('#typing-scene .webchat__typing-indicator');
        const stockStyle = await stock.evaluate((el) => {
          const cs = getComputedStyle(el);
          return { image: cs.backgroundImage, bg: cs.backgroundColor, width: el.getBoundingClientRect().width };
        });
        expect(stockStyle.image).toBe('none');
        expect(stockStyle.bg).toBe('rgba(0, 0, 0, 0)');

        const visible = {
          shimmer: indicator.locator('.gt-typing__shimmer'),
          pulse: indicator.locator('.gt-typing__pulse'),
          dots: indicator.locator('.gt-typing__dots'),
        };
        for (const [name, locator] of Object.entries(visible)) {
          if (name === mode) await expect(locator).toBeVisible();
          else await expect(locator).toBeHidden();
        }
        if (mode === 'shimmer') await expect(visible.shimmer).toHaveText('Thinking…');
        if (mode === 'dots') {
          const bubbleBg = await visible.dots.evaluate((el) => getComputedStyle(el).backgroundColor);
          const expected = theme === 'dark' ? 'rgb(31, 41, 55)' : 'rgb(255, 255, 255)';
          expect(bubbleBg).toBe(expected);
        }
        await shoot(page, `default-${mode}-${theme}`);
      });
    }
  }

  test('an absent or unknown choice falls back to shimmer', async ({ page }) => {
    await open(page, 'default', 'typing-bogus');
    await expect(page.locator('html')).toHaveAttribute('data-typing-indicator', 'shimmer');
    const indicator = await mountTypingScene(page, 'default', 'light');
    await expect(indicator).toHaveAttribute('data-mode', 'shimmer');
  });

  test('the page URL overrides the tenant choice', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await webchat.installMockWebChat();
    await page.goto(`${webchat.fullscreenUrl({ skin: 'default', variant: 'typing-dots-override' })}&typingIndicator=pulse`);
    await webchat.expectChatReady();
    await expect(page.locator('html')).toHaveAttribute('data-typing-indicator', 'pulse');
  });

  for (const theme of ['light', 'dark'] as Theme[]) {
    test(`3aigent skin draws the default look (${theme})`, async ({ page }) => {
      await open(page, '3aigent', `typing-3aigent-${theme}`);
      const indicator = await mountTypingScene(page, '3aigent', theme);
      await expect(indicator).toHaveAttribute('data-mode', 'shimmer');
      await expect(indicator.locator('.gt-typing__shimmer')).toBeVisible();
      await shoot(page, `3aigent-shimmer-${theme}`);
    });
  }

  test('reduced motion stops the animation', async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await open(page, 'default', 'typing-dots-reduced');
    const indicator = await mountTypingScene(page, 'default', 'light');
    const animation = await indicator
      .locator('.gt-typing__dots > i')
      .first()
      .evaluate((el) => getComputedStyle(el).animationName);
    expect(animation).toBe('none');
  });
});
