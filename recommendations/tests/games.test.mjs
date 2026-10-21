import {test} from 'node:test';
import assert from 'node:assert/strict';
import {randomUUID,randomBytes} from 'node:crypto';
import {readFileSync,mkdtempSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {openStore} from '../src/store.mjs';
import {createApi} from '../src/api.mjs';
import {gameDirectory} from '../src/games.mjs';
import {digest,recipe} from '../src/contracts.mjs';
const entries=[{id:'Q1',zh:'示例游戏',en:'Example Game',keys:['steam:100','gog:200','epic_slug:example']},{id:'Q2',zh:'示例游戏 2',en:'Example Game 2',keys:['steam:101']}];
function fixture(t){
  const db=openStore(':memory:');t.after(()=>db.close());
  const api=createApi(db,{secret:'test-only'.repeat(5),now:()=>1800000000,sendCode:async()=>{throw Error('No mail in matching tests');},adminEmail:'admin@example.com',games:entries});
  const user=(name)=>{const token=randomBytes(32).toString('base64url'),id=randomUUID();db.prepare('INSERT INTO users(id,email,created) VALUES(?,?,?)').run(id,name+'@example.com',1);db.prepare('INSERT INTO sessions VALUES(?,?,?)').run(digest(token),id,1900000000);return {id,token};};
  const call=(method,path,body,who,version='4')=>api({method,url:new URL(path,'http://localhost'),body,headers:{'x-rtxfg-feedback':version,...(who?{authorization:'Bearer '+who.token}:{})}});
  return {db,call,owner:user('owner'),reader:user('reader'),admin:user('admin')};
}
const content=(id='gog:200',name='示例游戏')=>({schema:1,game:{id,name},title:'我的方案',notes:'DX12',scheme:'dlssg-transfusion-1.4.5.3',patchVersion:'1.1.4',entries:['dxgi.dll'],renderer:'dx12',gpuModel:'RTX 3060',driver:'581.80',gpuAlias:'unchanged',neuralRuntime:'none',platform:'gog',edition:'1.2'});
const post=(f,c=content())=>f.call('POST','/v1/recipes',{submission:randomUUID(),content:c},f.owner);
const rejects=(promise,code)=>assert.rejects(promise,e=>e.message===code);
test('Reviewed cross-store IDs share results, actions retain context, legacy clients retain exact scope',async t=>{
  const f=fixture(t),r=await post(f),path='/v1/recipes/'+r.id;
  const page=await f.call('GET','/v1/recipes?game=steam:100');assert.equal(page.total,1);assert.equal(page.items[0].content.game.id,'steam:100');assert.equal(page.items[0].content.platform,'gog');
  assert.equal((await f.call('GET','/v1/recipes?game=steam:101')).total,0);
  assert.equal((await f.call('GET','/v1/recipes?game=steam:100',null,null,'3')).total,0);
  const legacy=await f.call('GET',path,null,null,'3');assert.ok(!('platform' in legacy.content));assert.ok(!('edition' in legacy.content));
  const voted=await f.call('PUT',path+'/vote?game=steam:100',{revision:1,value:1},f.reader);assert.equal(voted.likes,1);assert.equal(voted.content.game.id,'steam:100');
  const edit=await f.call('PUT',path+'?game=steam:100',{revision:1,content:{...page.items[0].content,notes:'编辑后仍保留评价'}},f.owner);assert.equal(edit.likes,1);assert.equal(edit.revision,2);assert.equal(f.db.prepare('SELECT game FROM recipes WHERE id=?').get(r.id).game,'gog:200');
  await rejects(f.call('PUT',path+'/vote?game=steam:101',{revision:2,value:-1},f.reader),'game_mismatch');assert.equal((await f.call('GET',path)).likes,1);
  const oldEdit=await f.call('PUT',path,{revision:2,content:{...legacy.content,notes:'旧客户端编辑'}},f.owner,'3');assert.ok(!oldEdit.content.platform);assert.equal(JSON.parse(f.db.prepare('SELECT content FROM recipes WHERE id=?').get(r.id).content).platform,'gog');
});
test('Aliases share the five-item quota and cannot leak hidden game activity',async t=>{
  const f=fixture(t);for(let i=0;i<5;i++)await post(f,content(i%2?'gog:200':'steam:100'));
  await rejects(post(f,content('epic_slug:example')),'five_recipes_per_game');
  f.db.exec("UPDATE recipes SET status='hidden'");
  const results=await f.call('GET','/v1/games?q=Example');assert.equal(results.items.find(i=>i.id==='steam:100').recipes,0);
  assert.equal((await f.call('GET','/v1/admin/games?q=Example',null,f.admin)).items.find(i=>i.id==='steam:100').recipes,5);
});
test('Player names are review leads, admin grouping is reversible and preserves ownership, feedback and moderation',async t=>{
  const f=fixture(t),name='Example title from player',id='custom:'+digest(name.toLowerCase());
  const r=await post(f,content(id,name)),path='/v1/recipes/'+r.id;
  await f.call('PUT',path+'/vote',{revision:1,value:1},f.reader);
  await post(f,content('steam:101','示例游戏 2'));
  assert.equal((await f.call('GET','/v1/recipes?game=steam:100')).total,0);
  const preview='/v1/admin/game-associations/preview?'+new URLSearchParams({source:id,target:'steam:100'});
  await rejects(f.call('GET',preview,null,f.reader),'admin_required');
  const plan=await f.call('GET',preview,null,f.admin),body={source:id,target:'steam:100',revision:plan.revision};
  await f.call('POST','/v1/admin/game-associations',body,f.admin);
  const linked=await f.call('GET','/v1/recipes?game=steam:100');assert.equal(linked.total,1);assert.equal(linked.items[0].id,r.id);assert.equal(linked.items[0].likes,1);assert.equal(linked.items[0].author,f.owner.id);
  const original=await f.call('GET','/v1/recipes?game='+id);assert.equal(original.items[0].content.game.name,name);recipe(original.items[0].content);
  await rejects(f.call('POST','/v1/admin/game-associations',body,f.admin),'mapping_changed');
  await f.call('POST','/v1/admin/moderate',{id:r.id,action:'hide',revision:1},f.admin);
  assert.equal((await f.call('GET','/v1/recipes?game=steam:100')).total,0);
  const state=await f.call('GET','/v1/admin/games',null,f.admin);
  await f.call('POST','/v1/admin/game-associations/undo',{revision:state.revision},f.admin);
  assert.equal(f.db.prepare('SELECT status FROM recipes WHERE id=?').get(r.id).status,'hidden');
  assert.equal((await f.call('GET','/v1/recipes?game=steam:100&mine=1',null,f.owner)).total,0);
  assert.equal((await f.call('GET','/v1/recipes?game='+id+'&mine=1',null,f.owner)).items[0].likes,1);
  await rejects(f.call('POST','/v1/admin/game-associations/undo',{revision:state.revision},f.admin),'mapping_changed');
});
test('Catalog refresh preserves manual mappings; newer catalog changes disable stale undo',t=>{
  const db=openStore(':memory:');t.after(()=>db.close());let g=gameDirectory(db,entries);
  const p=g.preview('steam:101','steam:100');g.merge({source:p.source.id,target:p.target.id,revision:p.revision},'admin',1);
  g=gameDirectory(db,entries);assert.equal(g.resolve('steam:101'),'steam:100');
  const rev=g.history().revision;
  g=gameDirectory(db,[...entries,{id:'Q3',zh:'新游戏',en:'New game',keys:['steam:300']}]);assert.equal(g.resolve('steam:101'),'steam:100');
  assert.throws(()=>g.undo({revision:rev},'admin',2),/mapping_changed/);
  assert.throws(()=>g.undo({revision:g.history().revision},'admin',2),/mapping_not_reversible/);
});
test('Bundled directory is reproducible, valid and matches the client name catalog',t=>{
  const bundle=JSON.parse(readFileSync(new URL('../src/game-directory.json',import.meta.url),'utf8'));
  const source=JSON.parse(readFileSync(new URL('../../rust/assets/game-names.json',import.meta.url),'utf8'));
  assert.deepEqual(bundle.entries,source.entries);
  const db=openStore(':memory:');t.after(()=>db.close());const g=gameDirectory(db,bundle.entries);
  assert.equal(g.search('鸣潮')[0].id,'steam:3513350');assert.ok(g.search('Example').length<=20);
  assert.throws(()=>recipe({...content(),edition:'C:/Users/private'}));
});
test('Schema-three migration leaves posts and feedback unchanged',t=>{
  const dir=mkdtempSync(join(tmpdir(),'rtxfg-games-'));t.after(()=>rmSync(dir,{recursive:true,force:true}));const file=join(dir,'test.sqlite');let db=openStore(file);
  db.exec("INSERT INTO users VALUES('author','a@example.com',0,1);DROP TABLE game_keys;DROP TABLE game_mapping_changes;PRAGMA user_version=3;");db.close();
  db=openStore(file);assert.equal(db.prepare('PRAGMA user_version').get().user_version,4);assert.equal(db.prepare('SELECT COUNT(*) n FROM users').get().n,1);db.close();
});
