const encoder = new TextEncoder();
export class ApiError extends Error {
  constructor(status, code) { super(code); this.status = status; this.code = code; }
}
export const requireThat = (condition, code = 'invalid_request', status = 400) => {
  if (!condition) throw new ApiError(status, code);
};
export const hex = bytes => Array.from(new Uint8Array(bytes), b => b.toString(16).padStart(2, '0')).join('');
export const sha256 = async value => hex(await crypto.subtle.digest('SHA-256', typeof value === 'string' ? encoder.encode(value) : value));
export const unbase64 = value => {
  requireThat(typeof value === 'string' && value.length <= 1024 && /^[A-Za-z0-9+/]*={0,2}$/.test(value), 'invalid_base64');
  try { return Uint8Array.from(atob(value), x => x.charCodeAt(0)); } catch { throw new ApiError(400, 'invalid_base64'); }
};
export async function readBody(request) {
  requireThat(request.headers.get('content-type')?.split(';')[0] === 'application/json', 'json_required', 415);
  const reader = request.body?.getReader();
  requireThat(reader, 'body_required');
  let size = 0; const parts = [];
  while (true) {
    const { value, done } = await reader.read(); if (done) break;
    size += value.length;
    if (size > 32768) { void reader.cancel().catch(() => {}); throw new ApiError(413, 'body_too_large'); }
    parts.push(value);
  }
  const bytes = new Uint8Array(size); let offset = 0;
  for (const part of parts) { bytes.set(part, offset); offset += part.length; }
  let text, data;
  try { text = new TextDecoder('utf-8', { fatal: true }).decode(bytes); data = JSON.parse(text); }
  catch { throw new ApiError(400, 'invalid_json'); }
  requireThat(data && typeof data === 'object' && !Array.isArray(data));
  return { text, data };
}
export const signingText = (method, route, digest, timestamp, nonce, identity) =>
  ['RTXFG-COMMUNITY-V1', method, route, digest, timestamp, nonce, identity].join('\n');
export async function authenticate(request, body, publicKey, now = Math.floor(Date.now() / 1000)) {
  const timestamp = request.headers.get('x-rtx-timestamp') || '';
  const nonce = request.headers.get('x-rtx-nonce') || '';
  const identity = request.headers.get('x-rtx-identity') || '';
  requireThat(/^\d{10}$/.test(timestamp) && Math.abs(now - Number(timestamp)) <= 120, 'expired_request', 401);
  requireThat(/^[a-f0-9]{64}$/.test(identity) && /^[a-f0-9]{32}$/.test(nonce), 'invalid_identity', 401);
  const raw = unbase64(publicKey);
  requireThat(raw.length === 65 && raw[0] === 4 && await sha256(raw) === identity, 'key_mismatch', 401);
  const signature = unbase64(request.headers.get('x-rtx-signature') || '');
  requireThat(signature.length === 64, 'invalid_signature', 401);
  const url = new URL(request.url);
  const canonical = signingText(request.method, url.pathname + url.search, await sha256(body), timestamp, nonce, identity);
  try {
    const key = await crypto.subtle.importKey('raw', raw, { name: 'ECDSA', namedCurve: 'P-256' }, false, ['verify']);
    requireThat(await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, key, signature, encoder.encode(canonical)), 'invalid_signature', 401);
  } catch (error) { if (error instanceof ApiError) throw error; throw new ApiError(401, 'invalid_signature'); }
  return { identity, nonce, timestamp: Number(timestamp) };
}
export async function verifyTurnstile(token, env, hostname) {
  requireThat(typeof token === 'string' && token.length > 0 && token.length <= 2048, 'verification_required', 403);
  requireThat(env.TURNSTILE_SECRET && env.PUBLIC_HOST === hostname, 'service_not_configured', 503);
  const response = await fetch('https://challenges.cloudflare.com/turnstile/v0/siteverify', {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ secret: env.TURNSTILE_SECRET, response: token }),
  });
  requireThat(response.ok, 'verification_unavailable', 503);
  const result = await response.json();
  requireThat(result.success === true && result.hostname === hostname && result.action === 'register', 'verification_failed', 403);
}
// Only short plain text is accepted; there is no raw config, path or arbitrary
// hardware blob in the protocol. Render these strings as text, never HTML.
export function safeText(value, maximum = 120) {
  requireThat(typeof value === 'string' && value.length <= maximum && !/[\u0000-\u001f<>]/.test(value), 'invalid_text');
  requireThat(!/(?:[a-z]:[\\/]|\\\\|\/(?:home|users|tmp|var)\/|%[A-Za-z_]+%|https?:\/\/|\b[^\s@]+@[^\s@]+\.[^\s@]+\b)/i.test(value), 'private_data_rejected');
  return value.trim();
}
