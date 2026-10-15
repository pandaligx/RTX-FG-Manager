import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import api from '../src/worker.mjs';

test('only the verification page preserves its public origin for form callbacks', async () => {
  const env = { ASSETS: { fetch: async () => new Response('page') } };
  for (const path of ['/verify', '/verify.html', '/admin', '/help/']) {
    const response = await api.fetch(new Request('https://test.example' + path), env);
    assert.equal(response.headers.get('referrer-policy'), path.startsWith('/verify') ? 'origin' : 'no-referrer');
    assert.match(response.headers.get('content-security-policy'), /form-action http:\/\/127\.0\.0\.1:\*/);
  }
});

test('verification failure requires manual retry and expired sessions cannot submit', async () => {
  const source = await readFile(new URL('../public/verify.js', import.meta.url), 'utf8');
  const elements = Object.fromEntries(['status','finish','retry'].map(id => [id, {
    hidden: true, addEventListener(_, action) { this.click = action; }
  }]));
  let options, expire, resets = 0, submitted = 0;
  const turnstile = { render(_, value) { options = value; return 'test-widget'; },
    reset() { resets++; }, remove() {} };
  const context = { URLSearchParams, location: { pathname: '/verify', hash: '#port=45678&state=' + 'a'.repeat(64) },
    history: { replaceState() {} }, window: { turnstile },
    document: { querySelector: key => elements[key.slice(1)], body: { append() {} },
      createElement: () => ({ append() {}, submit() { submitted++; } }) },
    fetch: async () => ({ ok: true, json: async () => ({ siteKey: 'unit-test-only' }) }),
    setTimeout: callback => { expire = callback; return 1; }, clearTimeout() {} };
  vm.runInNewContext(source, context);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(options.retry, 'never');
  options['error-callback']('300010');
  assert.match(elements.status.textContent, /300010/);
  assert.equal(elements.retry.hidden, false);
  elements.finish.click();
  assert.equal(submitted, 0);
  elements.retry.click();
  assert.equal(resets, 1);
  assert.equal(elements.retry.hidden, true);
  options.callback('synthetic-unit-test-value');
  assert.equal(elements.finish.hidden, false);
  options['expired-callback']();
  elements.finish.click();
  assert.equal(submitted, 0);
  expire();
  options.callback('late-value');
  elements.retry.click();
  elements.finish.click();
  assert.equal(submitted, 0);
  assert.equal(resets, 1);
  assert.match(elements.status.textContent, /超时/);
});
