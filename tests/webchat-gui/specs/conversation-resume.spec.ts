import { expect, test, type Page } from '@playwright/test';
import { WebChatGuiPage } from '../pages/webchatGuiPage';

// botframework-directlinejs opens conversations through XMLHttpRequest, so the
// resume logic has to live in the XHR wrapper: the fetch wrapper never sees it.

const CONVERSATIONS = '**/v3/directline/conversations**';
const BASE = '/v1/messaging/webchat/default/v3/directline/conversations';
const CONVERSATION_ID = 'conv-resume-1';
const PAGE = '/v1/web/webchat/default/?tenant=default';

type Seen = { method: string; path: string; body: string | null; authorization: string | undefined };

const OWNER_REQUIRED = {
  error: 'forbidden',
  code: 'ConversationOwnerRequired',
  message: 'this conversation belongs to another session; start a new conversation',
};

async function mockDirectLine(page: Page, resume: { status: number; body?: unknown }) {
  const seen: Seen[] = [];
  await page.route(CONVERSATIONS, async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    seen.push({ method: request.method(), path, body: request.postData(), authorization: request.headers()['authorization'] });
    if (request.method() === 'POST') {
      await route.fulfill({
        status: 201,
        contentType: 'application/json',
        body: JSON.stringify({ conversationId: CONVERSATION_ID, token: 'conv-token-1', streamUrl: `ws://example.test/${CONVERSATION_ID}/stream` }),
      });
      return;
    }
    await route.fulfill({
      status: resume.status,
      contentType: 'application/json',
      body: JSON.stringify(
        resume.status === 200
          ? { conversationId: CONVERSATION_ID, token: 'reissued', streamUrl: `ws://example.test/${CONVERSATION_ID}/stream?fresh=1` }
          : (resume.body ?? { error: 'gone' }),
      ),
    });
  });
  return seen;
}

// Same call shape directlinejs makes: a JSON POST to /conversations.
async function openConversationViaXhr(page: Page) {
  await page.evaluate(
    (url) =>
      new Promise<number>((resolve) => {
        const xhr = new XMLHttpRequest();
        xhr.open('POST', url);
        xhr.setRequestHeader('Content-Type', 'application/json');
        xhr.setRequestHeader('Authorization', 'Bearer page-token');
        xhr.responseType = 'json';
        xhr.addEventListener('loadend', () => resolve(xhr.status));
        xhr.send(JSON.stringify({ user: { id: 'u1' } }));
      }),
    BASE,
  );
}

function savedConversation(page: Page) {
  return page.evaluate(() => {
    const key = Object.keys(localStorage).find((k) => k.startsWith('greentic:v2:dl:conversation:'));
    return key ? (JSON.parse(localStorage.getItem(key) as string) as { conversationId: string; streamUrl: string; timestamp: number }) : null;
  });
}

test.beforeEach(async ({ page }) => {
  await new WebChatGuiPage(page).installMockWebChat();
});

test.describe('conversation resume over XHR', () => {
  test('a new conversation is saved, and the next load resumes it with a GET', async ({ page }) => {
    const seen = await mockDirectLine(page, { status: 200 });
    await page.goto(PAGE);
    await openConversationViaXhr(page);

    expect((await savedConversation(page))?.conversationId).toBe(CONVERSATION_ID);
    expect(seen.map((s) => s.method)).toEqual(['POST']);

    await page.reload();
    await openConversationViaXhr(page);

    expect(seen[0].authorization).toBe('Bearer page-token');
    expect(seen.slice(1)).toEqual([
      { method: 'GET', path: `${BASE}/${CONVERSATION_ID}`, body: null, authorization: 'Bearer conv-token-1' },
    ]);
    const refreshed = await savedConversation(page);
    expect(refreshed?.conversationId).toBe(CONVERSATION_ID);
    expect(refreshed?.streamUrl).toContain('fresh=1');
  });

  test('a failed resume clears the saved conversation and reloads exactly once', async ({ page }) => {
    const seen = await mockDirectLine(page, { status: 404 });
    await page.goto(PAGE);
    await openConversationViaXhr(page);
    const conversationKey = await page.evaluate(() => Object.keys(localStorage).find((k) => k.startsWith('greentic:v2:dl:conversation:')) as string);
    await page.reload();

    let loads = 0;
    page.on('load', () => { loads += 1; });
    await openConversationViaXhr(page);
    await expect.poll(() => loads).toBe(1);
    expect(await savedConversation(page)).toBeNull();
    expect(seen.map((s) => s.method)).toEqual(['POST', 'GET']);

    // Re-seed a stale record: with the retry guard already set, a second
    // failure must clear it without reloading again.
    await page.evaluate((key) => {
      localStorage.setItem(key, JSON.stringify({ conversationId: 'stale', streamUrl: 'ws://x', timestamp: Date.now() }));
    }, conversationKey);
    await openConversationViaXhr(page);
    await page.waitForTimeout(1000);
    expect(loads).toBe(1);
    expect(await savedConversation(page)).toBeNull();
  });

  test('a saved record without a token resumes with the page token and fails closed', async ({ page }) => {
    const seen = await mockDirectLine(page, { status: 403, body: OWNER_REQUIRED });
    await page.goto(PAGE);
    await openConversationViaXhr(page);
    await page.evaluate(() => {
      const key = Object.keys(localStorage).find((k) => k.startsWith('greentic:v2:dl:conversation:')) as string;
      const record = JSON.parse(localStorage.getItem(key) as string);
      delete record.token;
      localStorage.setItem(key, JSON.stringify(record));
    });
    await page.reload();

    let loads = 0;
    page.on('load', () => { loads += 1; });
    await openConversationViaXhr(page);
    await expect.poll(() => loads).toBe(1);
    expect(await savedConversation(page)).toBeNull();
    expect(seen.slice(1).map((s) => [s.method, s.authorization])).toEqual([['GET', 'Bearer page-token']]);
  });

  test('an XHR reused after a resume sends its next request untouched', async ({ page }) => {
    const seen = await mockDirectLine(page, { status: 200 });
    await page.goto(PAGE);
    await openConversationViaXhr(page);
    await page.reload();

    await page.evaluate(
      ({ base, id }) =>
        new Promise<void>((resolve) => {
          const xhr = new XMLHttpRequest();
          xhr.open('POST', base);
          xhr.setRequestHeader('Authorization', 'Bearer page-token');
          xhr.addEventListener(
            'loadend',
            () => {
              xhr.open('POST', `${base}/${id}/activities`);
              xhr.setRequestHeader('Content-Type', 'application/json');
              xhr.setRequestHeader('Authorization', 'Bearer next-token');
              xhr.addEventListener('loadend', () => resolve(), { once: true });
              xhr.send(JSON.stringify({ type: 'message', text: 'hi' }));
            },
            { once: true },
          );
          xhr.send(JSON.stringify({ user: { id: 'u1' } }));
        }),
      { base: BASE, id: CONVERSATION_ID },
    );

    expect(seen.slice(1)).toEqual([
      { method: 'GET', path: `${BASE}/${CONVERSATION_ID}`, body: null, authorization: 'Bearer conv-token-1' },
      {
        method: 'POST',
        path: `${BASE}/${CONVERSATION_ID}/activities`,
        body: JSON.stringify({ type: 'message', text: 'hi' }),
        authorization: 'Bearer next-token',
      },
    ]);
  });
});
