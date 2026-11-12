import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { readFileSync } from 'node:fs';
import api from '../src/worker.mjs';
import { sha256, signingText } from '../src/security.mjs';
import { createIdentity, unlock, headers } from '../public/admin-identity.js';
const scheme='upstream-0.3.5-310-9', protocol='upstream040';
const b64 = bytes => Buffer.from(bytes).toString('base64');
test('non-Steam names are registered atomically by verified shares without mixing games', async () => {
  const env = environment(), a = await identity(env);
  const game = 'game:n-' + await sha256('example game');
  const first = await create(env, a, { game, gameName: ' Example  GAME ' });
  assert.equal(first.status, 201);
  const second = await create(env, a, { game, gameName: 'Example Game' });
  assert.equal(second.status, 201);
  assert.equal(env.db.prepare('SELECT count(*) AS n FROM games WHERE id=?').get(game).n, 1);
  assert.equal((await create(env,a,{game,gameName:'Another Game'})).status,400);
  assert.equal((await create(env,a,{game,gameName:'C:\\Users\\Player\\Game'})).status,400);
  assert.equal((await create(env,a,{game:'game:unknown'})).status,400);
  const other = 'game:n-' + await sha256('example game 2');
  assert.equal((await create(env,a,{game:other,gameName:'Example Game 2'})).status,201);
  const response = await api.fetch(new Request('https://test.example/v1/profiles?game='+game),env);
  assert.deepEqual(new Set((await response.json()).profiles.map(p=>p.id)),new Set([first.id,second.id]));
  const bad = 'game:n-' + await sha256('bad game');
  assert.equal((await create(env,a,{game:bad,gameName:'Bad Game',content:{...content(),settings:{LogPath:'private'}}})).status,400);
  assert.equal(env.db.prepare('SELECT id FROM games WHERE id=?').get(bad),undefined);
  const anonymous = await api.fetch(new Request('https://test.example/v1/profiles',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({game,gameName:'Example Game',scheme,protocol,content:content()})}),env);
  assert.equal(anonymous.status,401);
});
function environment() {
  const db = new DatabaseSync(':memory:');
  db.exec(readFileSync(new URL('../migrations/0001_community.sql',import.meta.url),'utf8'));
  db.exec(readFileSync(new URL('../migrations/0002_moderator_signatures.sql',import.meta.url),'utf8'));
  function prepare(sql) {
    let args=[];
    return { bind(...values){args=values;return this;},
      async first(){ return db.prepare(sql).get(...args) || null; },
      async all(){return {results:db.prepare(sql).all(...args)};},
      run(){ const r=db.prepare(sql).run(...args);return {meta:{changes:r.changes}}; } };
  }
  return { DB:{prepare,async batch(statements){db.exec('BEGIN');try{const results=statements.map(s=>s.run());db.exec('COMMIT');return results;}catch(e){db.exec('ROLLBACK');throw e;}}} , db };
}
async function identity(env) {
  const pair=await crypto.subtle.generateKey({name:'ECDSA',namedCurve:'P-256'},true,['sign','verify']);
  const pub=await crypto.subtle.exportKey('raw',pair.publicKey), id=await sha256(pub);
  env.db.prepare('INSERT INTO identities(id,public_key,verified_at) VALUES(?,?,?)').run(id,b64(pub),Math.floor(Date.now()/1000));
  return {pair,id};
}
async function request(env,person,method,path,data,overrides={}) {
  const raw=JSON.stringify(data), timestamp=String(Math.floor(Date.now()/1000)),nonce=crypto.randomUUID().replaceAll('-','');
  const message=signingText(method,path,await sha256(raw),timestamp,nonce,person.id);
  const signature=await crypto.subtle.sign({name:'ECDSA',hash:'SHA-256'},person.pair.privateKey,new TextEncoder().encode(message));
  const req=new Request('https://test.example'+path,{method,headers:{'content-type':'application/json','x-rtx-identity':person.id,'x-rtx-timestamp':timestamp,'x-rtx-nonce':nonce,'x-rtx-signature':b64(signature),...overrides},body:raw});
  return {req,response:await api.fetch(req.clone(),env)};
}
const content=()=>({title:'Balanced 1440p',notes:'Player supplied',settings:{optimized:'2',fg_engine:'DLSS'},hardware:{gpuModel:'RTX 3060',driver:'616.92'},patchVersion:'0.4.0'});
async function create(env,person,extra={}) {
  const r=await request(env,person,'POST','/v1/profiles',{game:'steam:1234',scheme,protocol,content:content(),...extra});
  return {status:r.response.status,...await r.response.json()};
}
async function browse(env) {
  const r=await api.fetch(new Request(`https://test.example/v1/profiles?game=steam:1234&scheme=${scheme}&protocol=${protocol}`),env);
  return r.json();
}

test('recommendations combine schemes only within the requested game',async()=>{
  const env=environment(),a=await identity(env);
  const first=await create(env,a,{content:{...content(),entry:'dxgi.dll'}});
  const second=await create(env,a,{scheme:'rtx40mfg-1.3.3-hf2',protocol:'rtxmfg_universal_133',
    content:{...content(),settings:{rtx_mode:'2'},patchVersion:'1.5.0',entry:'OptiScaler/plugins/MFGUnlocker.asi'}});
  assert.equal(first.status,201); assert.equal(second.status,201);
  assert.equal((await create(env,a,{game:'steam:5678'})).status,201);
  async function read(query){return api.fetch(new Request('https://test.example/v1/profiles?'+query),env);}
  const all=await (await read('game=steam:1234')).json();
  assert.deepEqual(new Set(all.profiles.map(p=>p.id)),new Set([first.id,second.id]));
  assert.ok(all.profiles.every(p=>p.game==='steam:1234'));
  assert.equal(all.profiles.find(p=>p.id===second.id).content.entry,'OptiScaler/plugins/MFGUnlocker.asi');
  assert.equal((await browse(env)).profiles.length,1); // Existing scoped clients stay isolated.
  assert.equal((await (await read('game=steam:5678')).json()).profiles.length,1);
  assert.equal((await read('')).status,400); // There is no global recommendation fallback.
  assert.equal((await read('game=steam:1234&scheme='+scheme)).status,400);
  const after=all.profiles[0].id;
  const page=await (await read('game=steam:1234&after='+after)).json();
  assert.ok(page.profiles.every(p=>p.game==='steam:1234'&&p.id>after));
});
test('loader metadata accepts reviewed entries, never private paths or another scheme plugin',async()=>{
  const env=environment(),a=await identity(env);
  for(const entry of ['C:\\Users\\private\\version.dll','../version.dll','OptiScaler/plugins/MFGUnlocker.asi','rtx-encore.asi','binkw64Hooked.dll','winhttp.dll']) {
    assert.equal((await create(env,a,{content:{...content(),entry}})).status,400,entry);
  }
  assert.equal((await create(env,a,{content:{...content(),entry:'dinput8.dll'}})).status,201);
});

test('hardware submissions contain only GPU model and driver version',async()=>{
  const env=environment(),a=await identity(env);
  for (const field of ['gpuFamily','vramMiB','width','height']) {
    const value={...content(),hardware:{...content().hardware,[field]:1234}};
    assert.equal((await create(env,a,{content:value})).status,400,field);
  }
  assert.equal((await create(env,a)).status,201);
  assert.deepEqual((await browse(env)).profiles[0].content.hardware,content().hardware);
});
test('five profile quota is enforced inside concurrent database transactions',async()=>{
  const env=environment(),a=await identity(env);
  const results=await Promise.all(Array.from({length:7},()=>create(env,a)));
  assert.equal(results.filter(r=>r.status===201).length,5);
  assert.equal(results.filter(r=>r.error==='five_profiles_per_game').length,2);
  assert.equal((await browse(env)).profiles.length,5);
});
test('signed bodies, replay protection, ownership and private-field rejection',async()=>{
  const env=environment(),a=await identity(env),b=await identity(env);
  const data={game:'steam:1234',scheme,protocol,content:content()};
  const {req,response}=await request(env,a,'POST','/v1/profiles',data);
  assert.equal(response.status,201);const {id}=await response.json();
  assert.equal((await api.fetch(req.clone(),env)).status,409);
  assert.equal((await request(env,a,'POST','/v1/profiles',data,{'x-rtx-signature':b64(new Uint8Array(64))})).response.status,401);
  assert.equal((await request(env,b,'DELETE',`/v1/profiles/${id}`,{revision:1})).response.status,403);
  assert.equal((await create(env,a,{content:{...content(),notes:'C:\\Users\\Someone\\game'}})).status,400);
  assert.equal((await create(env,a,{content:{...content(),settings:{LogPath:'secret'}}})).status,400);
  assert.equal((await create(env,a,{content:{...content(),official:true}})).status,400);
  assert.equal((await create(env,a,{protocol:'rtx_encore_json_v4'})).status,400);
});
test('votes are revision-specific, reversible; deleted profiles disappear without stale cache',async()=>{
  const env=environment(),a=await identity(env),b=await identity(env),p=await create(env,a);
  const path=`/v1/profiles/${p.id}`;
  assert.equal((await request(env,b,'POST',path+'/vote',{revision:1,kind:'works'})).response.status,200);
  assert.equal((await browse(env)).profiles[0].works,1);
  const state=await request(env,b,'POST',path+'/interactions',{revision:1});
  assert.deepEqual(await state.response.json(),{votes:['works'],reported:false});
  assert.equal((await request(env,b,'DELETE',path+'/vote',{revision:1,kind:'works'})).response.status,200);
  assert.equal((await browse(env)).profiles[0].works,0);
  await request(env,b,'POST',path+'/vote',{revision:1,kind:'works'});
  assert.equal((await request(env,a,'PUT',path,{revision:1,content:{...content(),title:'Updated'}})).response.status,200);
  assert.equal((await browse(env)).profiles[0].works,0);
  assert.equal((await request(env,a,'PUT',path,{revision:1,content:content()})).response.status,409);
  assert.equal((await request(env,b,'POST',path+'/interactions',{revision:1})).response.status,409);
  assert.deepEqual(await (await request(env,b,'POST',path+'/interactions',{revision:2})).response.json(),{votes:[],reported:false});
  assert.equal((await request(env,a,'DELETE',path,{revision:2})).response.status,200);
  assert.equal((await browse(env)).profiles.length,0);
  assert.equal(env.db.prepare('SELECT COUNT(*) n FROM revisions').get().n,0);
});
test('three distinct verified reporters hide; duplicate reports and author edits cannot bypass review',async()=>{
  const env=environment(),a=await identity(env),p=await create(env,a),path=`/v1/profiles/${p.id}`;
  const reporters=await Promise.all(Array.from({length:3},()=>identity(env)));
  for (let i=0;i<3;i++) {
    const payload={revision:1,reason:'Does not match this game'};
    assert.equal((await request(env,reporters[i],'POST',path+'/report',payload)).response.status,200);
    if(i===0){await request(env,reporters[i],'POST',path+'/report',payload);assert.equal((await browse(env)).profiles.length,1);}
  }
  assert.equal((await browse(env)).profiles.length,0);
  assert.equal((await request(env,a,'PUT',path,{revision:1,content:content()})).response.status,200);
  assert.equal((await browse(env)).profiles.length,0);
  assert.equal((await request(env,reporters[0],'DELETE',path+'/report',{revision:2})).response.status,200);
  assert.equal(env.db.prepare('SELECT COUNT(*) n FROM reports').get().n,2);
  assert.equal((await browse(env)).profiles.length,0);
  assert.equal((await api.fetch(new Request('https://test.example/v1/moderation'),env)).status,503);
});
test('invalid and excessive JSON requests are bounded',async()=>{
  const env=environment(),a=await identity(env);
  const huge=await request(env,a,'POST','/v1/profiles',{large:'x'.repeat(33000)});
  assert.equal(huge.response.status,413);
  for(let i=0;i<30;i++) assert.equal((await request(env,a,'POST','/v1/mine',{})).response.status,200);
  assert.equal((await request(env,a,'POST','/v1/mine',{})).response.status,429);
});

test('administrator encrypted identity, allowlist, signed reads, revocation and replay',async()=>{
  const env=environment(),author=await identity(env),p=await create(env,author);
  const session=await createIdentity('a-long-moderator-password');
  const restored=await unlock(session.backup,'a-long-moderator-password');
  await assert.rejects(unlock(session.backup,'wrong-long-password'));
  const tampered={...session.backup,identity:'0'.repeat(64)};
  await assert.rejects(unlock(tampered,'a-long-moderator-password'));
  env.MODERATOR_KEYS=JSON.stringify({[session.backup.identity]:session.backup.publicKey});
  assert.equal((await api.fetch(new Request('https://test.example/v1/moderation'),env)).status,401);
  const route='/v1/moderation',body=JSON.stringify({action:'hide',target:p.id,reason:'Moderator review'});
  const signed=await headers(restored,'POST',route,body);
  const req=new Request('https://test.example'+route,{method:'POST',headers:{...signed,'content-type':'application/json'},body});
  assert.equal((await api.fetch(req.clone(),env)).status,200);
  assert.equal((await api.fetch(req.clone(),env)).status,409);
  assert.equal((await browse(env)).profiles.length,0);
  assert.equal(env.db.prepare('SELECT COUNT(*) n FROM audit').get().n,1);
  const getHeaders=await headers(restored,'GET',route);
  const listing=await api.fetch(new Request('https://test.example'+route,{headers:getHeaders}),env);
  assert.equal(listing.status,200);assert.equal((await listing.json()).profiles.length,1);
  env.MODERATOR_KEYS=JSON.stringify({['1'.repeat(64)]:session.backup.publicKey});
  assert.equal((await api.fetch(new Request('https://test.example'+route,{headers:await headers(restored,'GET',route)}),env)).status,401);
});
test('only provisioned moderators register games, with audit and safe display names',async()=>{
  const env=environment(),person=await identity(env),session=await createIdentity('another-long-moderator-password');
  env.MODERATOR_KEYS=JSON.stringify({[session.backup.identity]:session.backup.publicKey});
  const body={action:'register_game',target:'game:test-fixture',name:'Explicit Test Fixture',reason:'Offline test catalog'};
  assert.equal((await request(env,person,'POST','/v1/moderation',body)).response.status,401);
  async function register(data){const text=JSON.stringify(data),route='/v1/moderation';return api.fetch(new Request('https://test.example'+route,{method:'POST',headers:{...await headers(session,'POST',route,text),'content-type':'application/json'},body:text}),env);}
  assert.equal((await register(body)).status,200);
  assert.equal((await register({...body,name:'Updated Test Fixture'})).status,200);
  const r=await api.fetch(new Request('https://test.example/v1/games'),env);
  assert.deepEqual((await r.json()).games,[{id:'game:test-fixture',name:'Updated Test Fixture'}]);
  assert.equal(env.db.prepare('SELECT COUNT(*) n FROM audit').get().n,2);
  assert.equal((await register({...body,target:'../outside'})).status,400);
  assert.equal((await register({...body,name:'C:\\Users\\private'})).status,400);
  assert.equal(env.db.prepare('SELECT COUNT(*) n FROM profiles').get().n,0);
});
