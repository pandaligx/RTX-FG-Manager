import { exact, gameId, need, text } from './contracts.mjs';
import { transaction } from './store.mjs';

// Only bundled reviewed mappings or an administrator can associate identities.
// Player names are suggestions, never automatic cross-title matching rules.
export function gameDirectory(db, entries = []) {
  const key = db.prepare('SELECT * FROM game_keys WHERE id=?');
  const resolve = id => key.get(gameId(id))?.canonical || id;
  const revision = () => Number(db.prepare("SELECT value FROM service_state WHERE key='game_directory_revision'").get()?.value || 0);
  const bump = () => db.prepare("INSERT INTO service_state(key,value) VALUES('game_directory_revision',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value").run(String(revision()+1));
  transaction(db,()=>{
    const insert=db.prepare('INSERT OR IGNORE INTO game_keys(id,canonical,name,original) VALUES(?,?,?,?)');
    let changed=false;
    for(const entry of entries) {
      const canonical=entry.keys.find(k=>k.startsWith('steam:')) || 'catalog:'+entry.id.toLowerCase();
      gameId(canonical);text(entry.zh,120);text(entry.en,160);
      // A later data deployment must not overwrite an administrator's grouping.
      const target=resolve(canonical);
      for(const id of [...entry.keys,'catalog:'+entry.id.toLowerCase(),canonical]) changed=!!insert.run(gameId(id),target,entry.zh,entry.en).changes || changed;
    }
    if(changed)bump();
  });
  const ids = id => {
    const canonical=resolve(id);
    return [...new Set([id,canonical,...db.prepare('SELECT id FROM game_keys WHERE canonical=?').all(canonical).map(r=>r.id)])];
  };
  const known = id => key.get(id) || db.prepare("SELECT game id,game canonical,json_extract(content,'$.game.name') name,'' original FROM recipes WHERE game=? AND status!='deleted' LIMIT 1").get(id);
  const describe = (id,admin=false) => {
    const canonical=resolve(id), data=known(canonical)||known(id);
    const group=ids(id);
    const recipes=Number(db.prepare(`SELECT COUNT(*) n FROM recipes r JOIN users u ON u.id=r.author WHERE r.game IN (${group.map(()=>'?')}) AND r.status!='deleted' ${admin?'':"AND r.status='visible' AND u.blocked=0"}`).get(...group).n);
    return {id:canonical,name:data?.name || canonical,original:data?.original || '',keys:group,recipes};
  };
  const search=(query,admin=false)=>{
    const q=text(query,120,true), pattern='%'+q.replace(/[\\%_]/g,c=>'\\'+c)+'%';
    need(q.length>=2 || admin,'query_too_short');
    const records=db.prepare(`SELECT DISTINCT r.game id FROM recipes r JOIN users u ON u.id=r.author
      WHERE r.status!='deleted' ${admin?'':"AND r.status='visible' AND u.blocked=0"}
      AND (json_extract(r.content,'$.game.name') LIKE ? ESCAPE '\\' OR r.game=?) ORDER BY r.updated DESC LIMIT 40`).all(pattern,q);
    const catalog=db.prepare("SELECT id FROM game_keys WHERE name LIKE ? ESCAPE '\\' OR original LIKE ? ESCAPE '\\' OR id=? ORDER BY name LIMIT 100").all(pattern,pattern,q);
    return [...new Set([...records,...catalog].map(r=>resolve(r.id)))].slice(0,20).map(id=>describe(id,admin));
  };
  const preview=(source,target)=>{
    source=gameId(source);target=gameId(target);
    need(known(source)&&known(target),'game_not_found',404);
    need(resolve(source)!==resolve(target),'games_already_linked',409);
    return {source:describe(source,true),target:describe(target,true),revision:revision()};
  };
  const merge=(body,actor,now)=>transaction(db,()=>{
    exact(body,['source','target','revision']);need(body.revision===revision(),'mapping_changed',409);
    const plan=preview(body.source,body.target), before=plan.source.keys.map(id=>({id,row:key.get(id)||null}));
    const put=db.prepare('INSERT INTO game_keys(id,canonical,name,original) VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET canonical=excluded.canonical');
    for(const item of before)put.run(item.id,plan.target.id,item.row?.name || plan.source.name,item.row?.original || '');
    bump();
    db.prepare('INSERT INTO game_mapping_changes(source,target,changes,created,actor) VALUES(?,?,?,?,?)').run(plan.source.id,plan.target.id,JSON.stringify({before,revision:revision()}),now,actor);
    db.prepare("INSERT INTO audit(actor,action,target,created) VALUES(?,'associate_games',?,?)").run(actor,plan.source.id+' -> '+plan.target.id,now);
    return {ok:true};
  });
  const undo=(body,actor,now)=>transaction(db,()=>{
    exact(body,['revision']);need(body.revision===revision(),'mapping_changed',409);
    const last=db.prepare('SELECT * FROM game_mapping_changes ORDER BY id DESC LIMIT 1').get();
    need(last && !last.undone,'mapping_not_reversible',409);
    const change=JSON.parse(last.changes);
    need(change.revision===revision(),'mapping_not_reversible',409);
    for(const item of change.before) {
      if(item.row)db.prepare('UPDATE game_keys SET canonical=?,name=?,original=? WHERE id=?').run(item.row.canonical,item.row.name,item.row.original,item.id);
      else db.prepare('DELETE FROM game_keys WHERE id=?').run(item.id);
    }
    db.prepare('UPDATE game_mapping_changes SET undone=1 WHERE id=?').run(last.id);
    bump();
    db.prepare("INSERT INTO audit(actor,action,target,created) VALUES(?,'undo_game_association',?,?)").run(actor,String(last.id),now);
    return {ok:true};
  });
  return {resolve,ids,describe,search,preview,merge,undo,label:id=>known(id)?.name || id,history:()=>({revision:revision(),history:db.prepare('SELECT id,source,target,created,undone FROM game_mapping_changes ORDER BY id DESC LIMIT 10').all()})};
}
