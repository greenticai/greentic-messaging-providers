import { expect, test } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import { WebChatGuiPage } from '../pages/webchatGuiPage';

const skinRoot = path.resolve(process.cwd(), 'packs/messaging-webchat-gui/assets/webchat-gui/skins');

// Every skin this pack ships, plus the scaffold new skins are copied from.
const styleOptionFiles = [
  'default/webchat/styleOptions.json',
  '3aigent/webchat/styleOptions.json',
  '3aigent/webchat/styleOptions-light.json',
  '3aigent/webchat/styleOptions-dark.json',
  '_template/webchat/styleOptions.json',
];

// The upload route's allow-list (components/messaging-provider-webchat/src/directline/upload.rs).
const ALLOWED_TYPES = [
  'image/jpeg',
  'image/png',
  'image/gif',
  'image/webp',
  'application/pdf',
  'text/plain',
  'text/markdown',
  'text/csv',
  'application/json',
];

function readOptions(file: string): Record<string, unknown> {
  return JSON.parse(fs.readFileSync(path.join(skinRoot, file), 'utf8'));
}

test.describe('file upload', () => {
  test('no shipped skin hides the upload button', () => {
    for (const file of styleOptionFiles) {
      expect(readOptions(file).hideUploadButton, file).toBe(false);
    }
  });

  test('the picker offers only what the upload route accepts', () => {
    for (const file of styleOptionFiles) {
      const accept = readOptions(file).uploadAccept;
      expect(typeof accept, file).toBe('string');
      const entries = String(accept).split(',').map((entry) => entry.trim());
      for (const type of ALLOWED_TYPES) {
        expect(entries, `${file} offers ${type}`).toContain(type);
      }
      for (const entry of entries) {
        const isExtension = /^\.[a-z]+$/.test(entry);
        expect(isExtension || ALLOWED_TYPES.includes(entry), `${file}: ${entry}`).toBe(true);
      }
    }
  });

  test('no shipped skin embeds an image thumbnail in the upload activity', () => {
    // Web Chat puts a data: URL thumbnail into the `activity` part, which the
    // upload route caps at 64 KiB; one photo thumbnail can exceed it.
    for (const file of styleOptionFiles) {
      expect(readOptions(file).enableUploadThumbnail, file).toBe(false);
    }
  });

  test('the SPA hands WebChat a config that shows the upload button', async ({ page }) => {
    const webchat = new WebChatGuiPage(page);
    await webchat.installMockWebChat();
    await webchat.openFullscreen({ skin: 'default', nav: false, login: false });
    await webchat.expectChatReady();
    const styleOptions = await page.evaluate(
      () =>
        (
          window as unknown as {
            __lastWebChatConfig?: {
              styleOptions?: { hideUploadButton?: boolean; enableUploadThumbnail?: boolean; uploadAccept?: string };
            };
          }
        ).__lastWebChatConfig?.styleOptions,
    );
    expect(styleOptions?.hideUploadButton).toBe(false);
    expect(styleOptions?.enableUploadThumbnail).toBe(false);
    expect(styleOptions?.uploadAccept).toContain('application/pdf');
  });
});
