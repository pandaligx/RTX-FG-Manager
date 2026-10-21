import { test } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { openStore } from '../src/store.mjs';
import { createApi } from '../src/api.mjs';
import { httpServer } from '../src/http.mjs';
import { recipe, digest } from '../src/contracts.mjs';

const content=()=>({schema:1,game:{id:'steam:292030',name:'巫师 3'},title:'DX12 推荐',notes:'先关闭游戏再安装。',
  scheme:'dlssg-transfusion-1.4.5.3',patchVersion:'1.1.4',entries:['dxgi.dll'],renderer:'dx12',gpuModel:'NVIDIA GeForce RTX 3060',driver:'581.80',gpuAlias:'unchanged',neuralRuntime:'none'});
function fixture(t,extra={}) {
  const db=openStore(':memory:');t.after(()=>db.close());let clock=1800000000;const mail=[];
  const api=createApi(db,{secret:'unit-test-'.repeat(5),now:()=>clock,sendCode:async(email,code)=>mail.push({email,code}),adminEmail:'admin@example.com',...extra});
  const call=(method,path,body,token,ip='test-ip',feedback=false)=>api({method,url:new URL(path,'http://localhost'),body,headers:{...(token?{authorization:'Bearer '+token}:{}),...(feedback?{'x-rtxfg-feedback':feedback===3?'3':'2'}:{})},ip});
  const login=async(email='player@example.com')=>{const c=await call('POST','/v1/auth/code',{email});const m=mail.at(-1);return call('POST','/v1/auth/verify',{challenge:c.challenge,code:m.code});};
  return {db,mail,call,login,tick:s=>{clock+=s;}};
}
const reject=(promise,code)=>assert.rejects(promise,e=>e.message===code);
test('OTP is one-use, bound to challenge, expires, and five wrong attempts persist',async t=>{
  const f=fixture(t);const c=await f.call('POST','/v1/auth/code',{email:'A@EXAMPLE.COM'}),code=f.mail[0].code;
  assert.equal(f.mail[0].email,'a@example.com');assert.ok(!JSON.stringify(f.db.prepare('SELECT * FROM codes').get()).includes(code));
  for(let i=0;i<5;i++)await reject(f.call('POST','/v1/auth/verify',{challenge:c.challenge,code:code==='000000'?'111111':'000000'}),'invalid_or_expired_code');
  await reject(f.call('POST','/v1/auth/verify',{challenge:c.challenge,code}),'invalid_or_expired_code');
  f.tick(61);const d=await f.call('POST','/v1/auth/code',{email:'a@example.com'});
  const good=await f.call('POST','/v1/auth/verify',{challenge:d.challenge,code:f.mail.at(-1).code});assert.equal(good.token.length,43);
  await reject(f.call('POST','/v1/auth/verify',{challenge:d.challenge,code:f.mail.at(-1).code}),'invalid_or_expired_code');
  await f.call('POST','/v1/auth/logout',{},good.token);await reject(f.call('GET','/v1/me',null,good.token),'session_expired');
  const e=await f.call('POST','/v1/auth/code',{email:'b@example.com'});f.tick(601);await reject(f.call('POST','/v1/auth/verify',{challenge:e.challenge,code:f.mail.at(-1).code}),'invalid_or_expired_code');
});
test('Mail failure revokes code and quotas bound concurrent delivery',async t=>{
  const f=fixture(t,{sendCode:async()=>{throw Error('SMTP credential must not escape');}});
  await reject(f.call('POST','/v1/auth/code',{email:'a@example.com'}),'mail_unavailable');assert.equal(f.db.prepare('SELECT COUNT(*) n FROM codes').get().n,0);
  const g=fixture(t);const results=await Promise.allSettled(Array.from({length:3},()=>g.call('POST','/v1/auth/code',{email:'b@example.com'})));
  assert.equal(results.filter(v=>v.status==='fulfilled').length,1);assert.equal(g.mail.length,1);
});
test('Anonymous browse, strict game isolation, owner controls, five-item cap and idempotency',async t=>{
  const f=fixture(t),a=await f.login(),b=await f.login('b@example.com');const submission=randomUUID();
  const r=await f.call('POST','/v1/recipes',{submission,content:content()},a.token);
  assert.equal((await f.call('POST','/v1/recipes',{submission,content:content()},a.token)).id,r.id);
  await reject(f.call('POST','/v1/recipes',{submission,content:{...content(),title:'不同'}},a.token),'submission_conflict');
  assert.equal((await f.call('GET','/v1/recipes?game=steam:292030')).total,1);
  assert.equal((await f.call('GET','/v1/recipes?game=steam:100')).total,0);
  assert.ok(!JSON.stringify(await f.call('GET','/v1/recipes?game=steam:292030')).includes('example.com'));
  await reject(f.call('DELETE','/v1/recipes/'+r.id,{revision:1},b.token),'not_owner');
  const creates=await Promise.allSettled(Array.from({length:5},()=>f.call('POST','/v1/recipes',{submission:randomUUID(),content:content()},a.token)));
  assert.equal(creates.filter(v=>v.status==='fulfilled').length,4);
  await f.call('DELETE','/v1/recipes/'+r.id,{revision:1},a.token);
  await reject(f.call('GET','/v1/recipes/'+r.id),'not_found');
  await f.call('POST','/v1/recipes',{submission:randomUUID(),content:content()},a.token);
});
test('Likes are unique and reversible, survive edits; hidden content stays hidden after edits',async t=>{
  const f=fixture(t),a=await f.login(),b=await f.login('b@example.com'),admin=await f.login('admin@example.com');
  const r=await f.call('POST','/v1/recipes',{submission:randomUUID(),content:content()},a.token),path='/v1/recipes/'+r.id;
  await reject(f.call('PUT',path+'/like',{revision:1,liked:true},a.token),'own_recipe_like');
  for(let i=0;i<2;i++)assert.equal((await f.call('PUT',path+'/like',{revision:1,liked:true},b.token)).likes,1);
  assert.equal((await f.call('PUT',path+'/like',{revision:1,liked:false},b.token)).likes,0);
  await f.call('PUT',path+'/like',{revision:1,liked:true},b.token);
  const next=await f.call('PUT',path,{revision:1,content:{...content(),notes:'新修订'}},a.token);assert.equal(next.likes,1);assert.equal(next.revision,2);
  await reject(f.call('PUT',path+'/like',{revision:1,liked:true},b.token),'revision_changed');
  await f.call('PUT',path+'/report',{reason:'需要核实'},b.token);
  await reject(f.call('POST','/v1/admin/moderate',{id:r.id,action:'hide'},b.token),'admin_required');
  await f.call('POST','/v1/admin/moderate',{id:r.id,action:'hide'},admin.token);
  await f.call('PUT',path,{revision:2,content:content()},a.token);
  assert.equal((await f.call('GET','/v1/recipes?game=steam:292030')).total,0);
  await f.call('POST','/v1/admin/moderate',{id:r.id,action:'restore'},admin.token);
  assert.equal((await f.call('GET','/v1/recipes?game=steam:292030')).total,1);
  await f.call('POST','/v1/admin/moderate',{id:r.id,action:'block_author'},admin.token);
  await reject(f.call('GET','/v1/me',null,a.token),'session_expired');
  assert.equal((await f.call('GET','/v1/recipes?game=steam:292030')).total,0);
});
test('Public contract rejects paths, injected fields, schemes and loader combinations',()=>{
  for(const mutate of [c=>c.path='C:\\Users\\person',c=>c.notes='C:\\Users\\person',c=>c.entries=['../../hack.dll'],c=>c.scheme='native-0.2.6-stable',c=>c.entries=['dxgi.dll','rtx-encore.asi'],c=>c.game.id='steam:0',c=>c.gpuAlias='whatever',c=>c.notes='<script>x</script>']){
    const c=content();mutate(c);assert.throws(()=>recipe(c));
  }
  const c=content();c.game={id:'custom:'+digest('my game'),name:'My Game'};assert.equal(recipe(c).game.id,c.game.id);
});

test('Feedback is exclusive, reversible, retained across edits and legacy-compatible',async t=>{
  const f=fixture(t),a=await f.login(),b=await f.login('b@example.com');
  const r=await f.call('POST','/v1/recipes',{submission:randomUUID(),content:content()},a.token),path='/v1/recipes/'+r.id;
  assert.equal(Object.hasOwn(r,'dislikes'),false);
  const vote=value=>f.call('PUT',path+'/vote',{revision:1,value},b.token,'test-ip',true);
  await f.call('PUT',path+'/like',{revision:1,liked:true},b.token);
  let c=await vote(-1);assert.equal(c.likes,0);assert.equal(c.dislikes,1);assert.equal(c.disliked,true);
  c=await vote(-1);assert.equal(c.dislikes,1);
  c=await vote(1);assert.equal(c.likes,1);assert.equal(c.dislikes,0);assert.equal(c.liked,true);
  c=await vote(0);assert.equal(c.likes+c.dislikes,0);
  await reject(f.call('PUT',path+'/vote',{revision:1,value:-1},a.token),'own_recipe_like');
  await reject(vote(2),'invalid_vote');
  await vote(-1);
  c=await f.call('PUT',path,{revision:1,content:{...content(),notes:'新版本'}},a.token,'test-ip',true);
  assert.equal(c.dislikes,1);assert.equal(c.revision,2);
  await reject(vote(1),'revision_changed');
  c=await f.call('PUT',path+'/vote',{revision:2,value:1},b.token,'test-ip',true);
  assert.equal(c.likes,1);assert.equal(c.dislikes,0);
  const legacy=await f.call('GET',path,null,b.token);assert.equal(Object.hasOwn(legacy,'disliked'),false);
  assert.equal(f.db.prepare('PRAGMA user_version').get().user_version,4);
});

test('Owners edit an existing full-quota share without duplication and may delete only their own',async t=>{
  const f=fixture(t),owner=await f.login(),other=await f.login('other@example.com');
  const cards=[];
  for(let i=0;i<5;i++)cards.push(await f.call('POST','/v1/recipes',{submission:randomUUID(),content:{...content(),title:'方案 '+i}},owner.token));
  const original=cards[0],path='/v1/recipes/'+original.id;
  const edited={...original.content,title:'修正后的方案',notes:'重新测试后的说明',entries:['version.dll'],gpuAlias:'rtx4060'};
  await reject(f.call('PUT',path,{revision:1,content:edited},other.token),'not_owner');
  await reject(f.call('PUT',path,{revision:1,content:{...edited,game:{id:'steam:100',name:'其他游戏'}}},owner.token),'game_cannot_change');
  const updated=await f.call('PUT',path,{revision:1,content:edited},owner.token);
  assert.equal(updated.id,original.id);assert.deepEqual(updated.content,edited);assert.equal(updated.revision,2);
  assert.equal((await f.call('GET','/v1/recipes?game=steam:292030&mine=1',null,owner.token)).total,5);
  await reject(f.call('PUT',path,{revision:1,content:edited},owner.token),'revision_changed');
  await reject(f.call('DELETE',path,{revision:2},other.token),'not_owner');
  await reject(f.call('DELETE',path,{revision:1},owner.token),'revision_changed');
  await f.call('DELETE',path,{revision:2},owner.token);
  assert.equal((await f.call('GET','/v1/recipes?game=steam:292030&mine=1',null,owner.token)).total,4);
});

test('Admin lists all, searches, reviews possible duplicates and deletes with conflict checks',async t=>{
  const f=fixture(t),a=await f.login(),b=await f.login('b@example.com'),admin=await f.login('admin@example.com');
  const create=c=>f.call('POST','/v1/recipes',{submission:randomUUID(),content:c},a.token);
  const first=await create(content()),second=await create({...content(),title:'同方案不同备注',notes:'需要比较'});
  await create({...content(),renderer:'vulkan',title:'另一接口'});
  await reject(f.call('GET','/v1/admin/recipes',null,b.token),'admin_required');
  assert.equal((await f.call('GET','/v1/admin/recipes',null,admin.token)).total,3);
  assert.equal((await f.call('GET','/v1/admin/recipes?filter=duplicates',null,admin.token)).total,2);
  assert.equal((await f.call('GET','/v1/admin/recipes?q='+encodeURIComponent('另一接口'),null,admin.token)).total,1);
  await f.call('PUT','/v1/recipes/'+first.id+'/vote',{revision:1,value:-1},b.token);
  await f.call('PUT','/v1/recipes/'+first.id+'/report',{reason:'核实'},b.token);
  assert.equal((await f.call('GET','/v1/admin/recipes?filter=reported',null,admin.token)).total,1);
  await reject(f.call('POST','/v1/admin/moderate',{id:first.id,action:'delete',revision:99},admin.token),'revision_changed');
  await f.call('POST','/v1/admin/moderate',{id:first.id,action:'delete',revision:1},admin.token);
  await reject(f.call('GET','/v1/recipes/'+first.id),'not_found');
  assert.equal(f.db.prepare('SELECT content FROM recipes WHERE id=?').get(first.id).content,'{}');
  assert.equal(f.db.prepare('SELECT COUNT(*) n FROM dislikes').get().n,0);
  assert.equal(f.db.prepare('SELECT COUNT(*) n FROM reports').get().n,0);
  assert.equal(f.db.prepare("SELECT COUNT(*) n FROM audit WHERE action='delete'").get().n,1);
  assert.equal((await f.call('GET','/v1/admin/recipes?filter=duplicates',null,admin.token)).total,0);
  assert.equal((await f.call('GET','/v1/recipes/'+second.id)).id,second.id);
});

test('Version-one database upgrade preserves accounts, recipes and positive feedback',t=>{
  const dir=mkdtempSync(join(tmpdir(),'rtxfg-migration-'));t.after(()=>rmSync(dir,{recursive:true,force:true}));
  const file=join(dir,'test.sqlite');let db=openStore(file);
  db.exec("INSERT INTO users VALUES('author','a@example.com',0,1),('reader','r@example.com',0,1)");
  db.prepare('INSERT INTO recipes(id,author,game,content,updated,submission) VALUES(?,?,?,?,?,?)').run('recipe','author','steam:292030',JSON.stringify(content()),1,'submission');
  db.exec("INSERT INTO likes VALUES('recipe',1,'reader'); DROP TABLE dislikes; DROP TABLE game_keys; DROP TABLE game_mapping_changes; ALTER TABLE reports DROP COLUMN active; ALTER TABLE reports DROP COLUMN reviewed_revision; PRAGMA user_version=1;");db.close();
  db=openStore(file);assert.equal(db.prepare('SELECT COUNT(*) n FROM likes').get().n,1);assert.equal(db.prepare('SELECT COUNT(*) n FROM users').get().n,2);db.close();
  db=openStore(file);assert.equal(db.prepare('SELECT COUNT(*) n FROM recipes').get().n,1);assert.equal(db.prepare('SELECT COUNT(*) n FROM dislikes').get().n,0);db.close();
});
test('HTTP rejects oversized bodies and foreign origins, does not leak backend errors',async t=>{
  const f=fixture(t),server=httpServer(f.call?async args=>{if(args.url.pathname==='/v1/fail')throw Error('secret');return {ok:true};}:null,{origin:'https://recommend.example.com'});
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));t.after(()=>server.close());const base='http://127.0.0.1:'+server.address().port;
  assert.equal((await fetch(base+'/v1/health',{headers:{Origin:'https://evil.example'}})).status,403);
  assert.equal((await fetch(base+'/v1/x',{method:'POST',headers:{'Content-Type':'application/json'},body:'x'.repeat(17000)})).status,413);
  assert.deepEqual(await (await fetch(base+'/v1/fail')).json(),{error:'internal_error'});
});

test('Reports are distinct, reversible, hide at three and remain reviewable after withdrawal',async t=>{
  const f=fixture(t),owner=await f.login(),admin=await f.login('admin@example.com'),readers=[];
  for(let i=0;i<3;i++)readers.push(await f.login(`reader${i}@example.com`));
  const r=await f.call('POST','/v1/recipes',{submission:randomUUID(),content:content()},owner.token),path='/v1/recipes/'+r.id;
  const report=(who,method='PUT')=>f.call(method,path+'/report',method==='PUT'?{reason:'误导信息'}:{},who.token,'test-ip',3);
  await reject(report(owner),'own_recipe_report');
  for(let i=0;i<3;i++)assert.equal((await report(readers[0])).status,'visible');
  assert.equal(f.db.prepare('SELECT COUNT(*) n FROM reports WHERE active=1').get().n,1);
  assert.equal((await report(readers[0],'DELETE')).reported,false);
  await report(readers[0]);await report(readers[1]);
  const hidden=await report(readers[2]);assert.equal(hidden.status,'hidden');assert.equal(hidden.reported,true);
  assert.equal((await f.call('GET','/v1/recipes?game=steam:292030')).total,0);
  assert.equal((await f.call('GET','/v1/recipes?game=steam:292030&reported=1',null,readers[0].token,'test-ip',3)).items[0].reported,true);
  await reject(f.call('GET','/v1/recipes?game=steam:292030&reported=1'),'login_required');
  await report(readers[2],'DELETE');assert.equal(f.db.prepare('SELECT status FROM recipes WHERE id=?').get(r.id).status,'hidden');
  await f.call('POST','/v1/admin/moderate',{id:r.id,action:'restore',revision:1},admin.token);
  await reject(report(readers[2]),'report_reviewed');
  await report(readers[0],'DELETE');await reject(report(readers[0]),'report_reviewed');
  assert.equal((await f.call('GET','/v1/admin/recipes?filter=reported',null,admin.token)).total,0);
  assert.equal(Object.hasOwn(await f.call('GET',path,null,readers[0].token,'test-ip',true),'reported'),false);
  await f.call('PUT',path,{revision:1,content:{...content(),notes:'新修订'}},owner.token);
  assert.equal((await report(readers[0])).status,'visible');
  assert.equal(f.db.prepare("SELECT COUNT(*) n FROM audit WHERE action='report_threshold_hide'").get().n,1);
});

test('Mail health hides details publicly and does not test SMTP on read',async t=>{
  let ready=false;
  const f=fixture(t,{mailHealth:()=>({ok:ready,state:ready?'ready':'smtp_unavailable',checkedAt:1000,lastAcceptedAt:500})});
  await reject(f.call('GET','/v1/health/mail'),'mail_unavailable');
  assert.equal((await f.call('GET','/v1/health')).ok,true);
  ready=true;assert.deepEqual(await f.call('GET','/v1/health/mail'),{ok:true});
  await reject(f.call('GET','/v1/admin/mail-health'),'admin_required');
  const admin=await f.login('admin@example.com');assert.equal((await f.call('GET','/v1/admin/mail-health',null,admin.token)).checkedAt,1000);
});
