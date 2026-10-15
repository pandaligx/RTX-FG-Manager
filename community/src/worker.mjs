import { ApiError, requireThat, readBody, authenticate, verifyTurnstile, safeText } from './security.mjs';
import { validateContent, validateRoute, validateGame } from './contracts.mjs';
import { moderator, moderatorKeys } from './moderator.mjs';

const json = (data, status = 200) => Response.json(data, { status, headers: {
  'cache-control': 'no-store', 'x-content-type-options': 'nosniff', 'access-control-allow-origin': '*',
} });
const idPattern = /^[0-9a-f-]{36}$/;
const query = (env, sql, ...args) => env.DB.prepare(sql).bind(...args);
const publicProfile = row => ({ id: row.id, author: row.author, game: row.game, scheme: row.scheme,
  protocol: row.protocol, revision: row.revision, official: !!row.official,
  updatedAt: row.updated_at, likes: row.likes || 0, works: row.works || 0, content: JSON.parse(row.content) });
const SELECT = `SELECT p.*,r.content,
 (SELECT COUNT(*) FROM votes v WHERE v.profile=p.id AND v.revision=p.revision AND v.kind='like') likes,
 (SELECT COUNT(*) FROM votes v WHERE v.profile=p.id AND v.revision=p.revision AND v.kind='works') works
 FROM profiles p JOIN revisions r ON r.profile=p.id AND r.revision=p.revision
 JOIN identities i ON i.id=p.author`;

async function signed(request, env, raw, publicKey) {
  const identity = request.headers.get('x-rtx-identity') || '';
  const person = await query(env, 'SELECT * FROM identities WHERE id=?', identity).first();
  requireThat(!person?.blocked, 'identity_blocked', 403);
  requireThat(publicKey || person, 'identity_not_registered', 401);
  return authenticate(request, raw, publicKey || person.public_key);
}
async function mutate(env, auth, statements, now) {
  const prefix = [
    query(env, 'INSERT INTO nonces(identity,nonce,expires_at) VALUES(?,?,?)', auth.identity, auth.nonce, now + 300),
    query(env, `INSERT INTO rate_buckets(identity,minute,count) VALUES(?,?,1)
      ON CONFLICT(identity,minute) DO UPDATE SET count=count+1`, auth.identity, Math.floor(now / 60)),
  ];
  return env.DB.batch([...prefix, ...statements]);
}
async function handle(request, env) {
  const url = new URL(request.url), path = url.pathname;
  const now = Math.floor(Date.now() / 1000);
  if (request.method === 'OPTIONS') return new Response(null, { status: 204, headers: {
    'access-control-allow-origin': '*', 'access-control-allow-methods': 'GET,POST,PUT,DELETE,OPTIONS',
    'access-control-allow-headers': 'content-type,x-rtx-identity,x-rtx-nonce,x-rtx-timestamp,x-rtx-signature',
    'access-control-max-age': '600',
  } });
  if (path === '/v1/health' && request.method === 'GET') return json({ api: 1, status: env.DB && env.TURNSTILE_SECRET && Object.keys(moderatorKeys(env)).length ? 'ready' : 'preparing' });
  if (!path.startsWith('/v1/')) {
    if (!env.ASSETS) return json({ error: 'not_found' },404);
    const asset=await env.ASSETS.fetch(request), response=new Response(asset.body,asset);
    // Form POSTs use this policy for Origin too. Preserve the public site origin
    // on the loopback callback so the native client's strict origin check works.
    // No path, state or token is included; all other pages keep no-referrer.
    response.headers.set('referrer-policy',path==='/verify'||path==='/verify.html'?'origin':'no-referrer');
    response.headers.set('x-content-type-options','nosniff');
    response.headers.set('content-security-policy',"default-src 'none'; script-src 'self' https://challenges.cloudflare.com; style-src 'self'; frame-src https://challenges.cloudflare.com; connect-src 'self' https://challenges.cloudflare.com; img-src 'self' data:; base-uri 'none'; form-action http://127.0.0.1:*; frame-ancestors 'none'");
    return response;
  }
  requireThat(env.DB, 'service_unavailable', 503);
  if (env.READ_LIMITER) {
    const { success } = await env.READ_LIMITER.limit({ key: request.headers.get('cf-connecting-ip') || 'local' });
    requireThat(success, 'rate_limited', 429);
  } else requireThat(!env.PUBLIC_HOST, 'service_not_configured', 503);
  if (path === '/v1/verification-config' && request.method === 'GET') {
    requireThat(env.TURNSTILE_SITE_KEY && env.TURNSTILE_SECRET,'service_not_configured',503);
    return json({siteKey:env.TURNSTILE_SITE_KEY});
  }
  if (request.method === 'GET' && path === '/v1/games') {
    const rows = await query(env, 'SELECT id,name FROM games ORDER BY name LIMIT 2000').all();
    return json({ games: rows.results });
  }
  if (request.method === 'GET' && path === '/v1/profiles') {
    const game = url.searchParams.get('game'), scheme = url.searchParams.get('scheme'), protocol = url.searchParams.get('protocol');
    validateGame(game);
    const scoped = scheme !== null || protocol !== null;
    if (scoped) validateRoute(game, scheme, protocol);
    const after = url.searchParams.get('after') || '';
    requireThat(after === '' || idPattern.test(after), 'invalid_cursor');
    // Client fetches a lightweight per-game index and ranks it locally. Hardware
    // is never sent to this browsing endpoint. Cursor is lexical, stable by ID.
    const rows = await query(env, `${SELECT} WHERE p.game=? ${scoped ? 'AND p.scheme=? AND p.protocol=?' : ''}
      AND p.status='visible' AND i.blocked=0 AND p.id>? ORDER BY p.id LIMIT 121`,
      ...(scoped ? [game, scheme, protocol, after] : [game, after])).all();
    const page = rows.results.slice(0,120);
    return json({ profiles: page.map(publicProfile), next: rows.results.length > 120 ? page.at(-1).id : null });
  }
  if (path.startsWith('/v1/moderation')) return moderation(request, env, now);
  requireThat(['POST','PUT','DELETE'].includes(request.method), 'not_found', 404);
  const origin = request.headers.get('origin');
  requireThat(!origin || origin === url.origin, 'origin_rejected', 403);
  const { text, data } = await readBody(request);
  if (request.method === 'POST' && path === '/v1/identities') {
    requireThat(Object.keys(data).every(k => ['publicKey','turnstileToken'].includes(k)), 'unsupported_field');
    const auth = await signed(request, env, text, data.publicKey);
    await verifyTurnstile(data.turnstileToken, env, url.hostname);
    await env.DB.batch([
      query(env, 'INSERT INTO identities(id,public_key,verified_at) VALUES(?,?,?) ON CONFLICT(id) DO NOTHING', auth.identity, data.publicKey, now),
      query(env, 'INSERT INTO nonces(identity,nonce,expires_at) VALUES(?,?,?)', auth.identity, auth.nonce, now + 300),
    ]);
    return json({ identity: auth.identity, verified: true }, 201);
  }
  const auth = await signed(request, env, text);
  if (path === '/v1/mine' && request.method === 'POST') {
    await mutate(env, auth, [], now);
    const rows = await query(env, `${SELECT} WHERE p.author=? ORDER BY p.updated_at DESC LIMIT 1000`, auth.identity).all();
    return json({ profiles: rows.results.map(row => ({ ...publicProfile(row), status: row.status })) });
  }
  if (path === '/v1/profiles' && request.method === 'POST') {
    requireThat(Object.keys(data).every(k => ['game','scheme','protocol','content'].includes(k)), 'unsupported_field');
    validateRoute(data.game, data.scheme, data.protocol);
    const content = validateContent(data.content, data.scheme, data.protocol);
    const game = await query(env, 'SELECT id FROM games WHERE id=?', data.game).first();
    requireThat(game || data.game.startsWith('steam:'), 'choose_known_game');
    const id = crypto.randomUUID();
    const statements = [];
    if (!game) statements.push(query(env, 'INSERT INTO games(id,name,created_at) VALUES(?,?,?) ON CONFLICT(id) DO NOTHING', data.game, 'Steam ' + data.game.slice(6), now));
    statements.push(
      query(env, 'INSERT INTO profiles(id,author,game,scheme,protocol,created_at,updated_at) VALUES(?,?,?,?,?,?,?)', id, auth.identity, data.game, data.scheme, data.protocol, now, now),
      query(env, 'INSERT INTO revisions(profile,revision,content,created_at) VALUES(?,1,?,?)', id, JSON.stringify(content), now),
    );
    await mutate(env, auth, statements, now);
    return json({ id, revision: 1 }, 201);
  }
  const match = path.match(/^\/v1\/profiles\/([0-9a-f-]{36})(?:\/(vote|report|interactions))?$/);
  requireThat(match, 'not_found', 404);
  const [, id, operation] = match;
  const profile = await query(env, 'SELECT * FROM profiles WHERE id=?', id).first();
  requireThat(profile, 'not_found', 404);
  if (!operation) {
    requireThat(profile.author === auth.identity, 'forbidden', 403);
    requireThat(data.revision === profile.revision, 'revision_conflict', 409);
    if (request.method === 'DELETE') {
      const results = await mutate(env, auth, [query(env, 'DELETE FROM profiles WHERE id=? AND author=? AND revision=?', id, auth.identity, data.revision)], now);
      requireThat(results.at(-1).meta.changes === 1, 'revision_conflict', 409);
      // No edge caching: subsequent reads query D1 and cannot serve deleted data.
      return json({ deleted: true });
    }
    requireThat(request.method === 'PUT', 'method_not_allowed', 405);
    requireThat(Object.keys(data).every(k => ['revision','content'].includes(k)), 'unsupported_field');
    const content = validateContent(data.content, profile.scheme, profile.protocol), revision = profile.revision + 1;
    await mutate(env, auth, [
      query(env, 'UPDATE profiles SET revision=?,updated_at=?,official=0 WHERE id=? AND author=? AND revision=?', revision, now, id, auth.identity, data.revision),
      // Concurrent same-revision requests collide on this primary key, rolling
      // back the entire batch, including its nonce and CAS update.
      query(env, 'INSERT INTO revisions(profile,revision,content,created_at) VALUES(?,?,?,?)', id, revision, JSON.stringify(content), now),
    ], now);
    return json({ id, revision, status: profile.status });
  }
  if (operation === 'interactions') {
    requireThat(request.method === 'POST', 'method_not_allowed', 405);
    requireThat(data.revision === profile.revision, 'revision_conflict', 409);
    await mutate(env, auth, [], now);
    const votes = await query(env,'SELECT kind FROM votes WHERE profile=? AND revision=? AND identity=?',id,profile.revision,auth.identity).all();
    const report = await query(env,'SELECT 1 FROM reports WHERE profile=? AND identity=?',id,auth.identity).first();
    return json({ votes:votes.results.map(v=>v.kind), reported:!!report });
  }
  requireThat(data.revision === profile.revision, 'revision_conflict', 409);
  requireThat(profile.author !== auth.identity, 'self_interaction_not_allowed', 403);
  requireThat(['POST','DELETE'].includes(request.method), 'method_not_allowed', 405);
  requireThat(request.method === 'DELETE' || profile.status === 'visible', 'profile_unavailable', 409);
  let statement;
  if (operation === 'vote') {
    requireThat(['like','works'].includes(data.kind), 'invalid_vote');
    statement = request.method === 'DELETE'
      ? query(env,'DELETE FROM votes WHERE profile=? AND revision=? AND identity=? AND kind=?',id,data.revision,auth.identity,data.kind)
      : query(env,'INSERT INTO votes(profile,revision,identity,kind) VALUES(?,?,?,?) ON CONFLICT DO NOTHING',id,data.revision,auth.identity,data.kind);
  } else {
    const reason = safeText(data.reason || '', 240);
    requireThat(request.method === 'DELETE' || reason.length > 0, 'reason_required');
    statement = request.method === 'DELETE'
      ? query(env,'DELETE FROM reports WHERE profile=? AND identity=?',id,auth.identity)
      : query(env,'INSERT INTO reports(profile,identity,revision,reason,created_at) VALUES(?,?,?,?,?) ON CONFLICT DO NOTHING',id,auth.identity,data.revision,reason,now);
  }
  await mutate(env, auth, [statement], now);
  return json({ applied: true });
}
async function moderation(request, env, now) {
  const path = new URL(request.url).pathname;
  requireThat(path==='/v1/moderation' && ['GET','POST'].includes(request.method),'not_found',404);
  const origin=request.headers.get('origin');
  requireThat(!origin || origin===new URL(request.url).origin,'origin_rejected',403);
  const body=request.method==='POST'?await readBody(request):{text:'',data:{}};
  const auth=await moderator(request,env,body.text), actor=auth.identity;
  const nonce=query(env,'INSERT INTO moderator_nonces(identity,nonce,expires_at) VALUES(?,?,?)',actor,auth.nonce,now+300);
  if (request.method === 'GET' && path === '/v1/moderation') {
    await nonce.run();
    const rows = await query(env, `${SELECT} WHERE p.status IN ('review','hidden') ORDER BY p.updated_at DESC LIMIT 100`).all();
    const reports = await query(env, 'SELECT * FROM reports ORDER BY created_at DESC LIMIT 500').all();
    return json({ profiles: rows.results.map(row => ({ ...publicProfile(row), status: row.status })), reports: reports.results });
  }
  requireThat(request.method === 'POST' && path === '/v1/moderation', 'not_found', 404);
  const { data } = body;
  if (data.action === 'register_game') {
    requireThat(Object.keys(data).every(k=>['action','target','reason','name'].includes(k)),'unsupported_field');
    requireThat(/^(steam:[1-9][0-9]{0,9}|game:[a-z0-9][a-z0-9-]{1,79})$/.test(data.target),'invalid_game');
    const name=safeText(data.name,120),reason=safeText(data.reason,240);
    requireThat(name.length>0 && reason.length>0,'name_and_reason_required');
    await env.DB.batch([nonce,
      query(env,'INSERT INTO games(id,name,created_at) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name',data.target,name,now),
      query(env,'INSERT INTO audit(actor,action,target,reason,created_at) VALUES(?,?,?,?,?)',actor,data.action,data.target,reason,now)]);
    return json({applied:true});
  }
  requireThat(Object.keys(data).every(k=>['action','target','reason'].includes(k)),'unsupported_field');
  requireThat(['hide','restore','official','unofficial','block','unblock'].includes(data.action), 'invalid_action');
  const reason = safeText(data.reason,240); requireThat(reason.length > 0, 'reason_required');
  let statement;
  if (data.action === 'block' || data.action === 'unblock') {
    requireThat(/^[a-f0-9]{64}$/.test(data.target), 'invalid_identity');
    statement = query(env,'UPDATE identities SET blocked=? WHERE id=?',data.action==='block'?1:0,data.target);
  } else {
    requireThat(idPattern.test(data.target), 'invalid_profile');
    statement = data.action === 'hide' || data.action === 'restore'
      ? query(env,'UPDATE profiles SET status=?,official=0 WHERE id=?',data.action==='hide'?'hidden':'visible',data.target)
      : query(env,"UPDATE profiles SET official=? WHERE id=? AND status='visible'",data.action==='official'?1:0,data.target);
  }
  const results=await env.DB.batch([nonce,statement,query(env,'INSERT INTO audit(actor,action,target,reason,created_at) SELECT ?,?,?,?,? WHERE changes()=1',actor,data.action,data.target,reason,now)]);
  requireThat(results[1].meta.changes===1,'target_not_found_or_unavailable',409);
  return json({ applied: true });
}
export default {
  async fetch(request, env) {
    try { return await handle(request, env); }
    catch (error) {
      if (error instanceof ApiError) return json({ error: error.code }, error.status);
      const text = String(error?.message || '');
      if (text.includes('profile_quota')) return json({ error: 'five_profiles_per_game' },409);
      if (text.includes('nonces.') || text.includes('revisions.')) return json({ error: 'replay_or_revision_conflict' },409);
      if (text.includes('count<=30')) return json({ error: 'rate_limited' },429);
      if (text.includes('identity_blocked')) return json({ error: 'identity_blocked' },403);
      // Never send SQL, private headers, stack traces or token values to clients.
      return json({ error: 'service_unavailable' },503);
    }
  },
  async scheduled(_event, env) {
    const now = Math.floor(Date.now()/1000);
    await env.DB.batch([
      query(env,'DELETE FROM nonces WHERE rowid IN (SELECT rowid FROM nonces WHERE expires_at<? LIMIT 5000)',now),
      query(env,'DELETE FROM moderator_nonces WHERE rowid IN (SELECT rowid FROM moderator_nonces WHERE expires_at<? LIMIT 5000)',now),
      query(env,'DELETE FROM rate_buckets WHERE rowid IN (SELECT rowid FROM rate_buckets WHERE minute<? LIMIT 5000)',Math.floor(now/60)-10),
    ]);
  },
};
