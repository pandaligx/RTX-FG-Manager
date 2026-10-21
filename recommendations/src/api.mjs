import { randomUUID } from 'node:crypto';
import { ApiError, exact, need, recipe, gameId, text, uuid } from './contracts.mjs';
import { authService } from './auth.mjs';
import { gameDirectory } from './games.mjs';
import { cleanExpired, limit, transaction, selectRecipes, publicRecipe, deleteRecipe } from './store.mjs';

export function createApi(db, options) {
  const now = options.now || (() => Math.floor(Date.now()/1000));
  const auth = authService(db, { ...options, now });
  const games=gameDirectory(db,options.games || []);
  let lastCleanup = 0;
  const lookup = id => db.prepare(selectRecipes + ' WHERE r.id=?').get(id);
  return async ({ method, url, headers = {}, body = {}, ip = 'local' }) => {
    const t = now(), route = url.pathname;
    if (t-lastCleanup>3600) { cleanExpired(db,t); lastCleanup=t; }
    limit(db,'read:' + auth.fingerprint(ip),t,60,180);
    if (method === 'GET' && route === '/v1/health') return { ok:true, protocol:1 };
    if (method === 'GET' && route === '/v1/health/mail') {
      need(options.mailHealth?.().ok,'mail_unavailable',503);
      return {ok:true};
    }
    if (method === 'POST' && route === '/v1/auth/code') return auth.request(body,ip);
    if (method === 'POST' && route === '/v1/auth/verify') return auth.verify(body,ip);
    const user = auth.user(headers.authorization, method!=='GET');
    // Older test clients reject unknown JSON fields. Opt in to the feedback extension.
    const feedback=headers['x-rtxfg-feedback'];
    const crossStore=feedback==='4';
    const context=url.searchParams.get('game');
    const card = (row,requested=context) => {
      const result=publicRecipe(db,row,user,['2','3','4'].includes(feedback),['3','4'].includes(feedback));
      if(crossStore && requested) {
        need(games.resolve(requested)===games.resolve(row.game),'game_mismatch');
        result.content.game={id:requested,name:games.describe(requested).name};
        // A custom ID is a hash of its exact name; keep its original public label.
        if(requested.startsWith('custom:')) result.content.game.name=games.label(requested);
      }
      if(!crossStore){delete result.content.platform;delete result.content.edition;}
      return result;
    };
    if (method === 'GET' && route === '/v1/me') { need(user,'login_required',401); return { id:user.id, admin:user.admin, expiresAt:user.expires }; }
    if (method === 'POST' && route === '/v1/auth/logout') { exact(body,[]); auth.logout(headers.authorization); return { ok:true }; }
    if (method === 'GET' && route === '/v1/recipes') {
      const game = gameId(url.searchParams.get('game'));
      const offset = Number(url.searchParams.get('offset') || 0);
      need(Number.isSafeInteger(offset) && offset>=0 && offset<=10000, 'invalid_page');
      const mine = url.searchParams.get('mine')==='1';
      const reported=url.searchParams.get('reported')==='1';
      need(!(mine && reported),'invalid_filter');
      need(!(mine || reported) || user,'login_required',401);
      const group=crossStore?games.ids(game):[game],scope=` WHERE r.game IN (${group.map(()=>'?')})`;
      const where = scope+(mine ? " AND r.author=? AND r.status!='deleted'" : reported ? " AND r.status!='deleted' AND EXISTS(SELECT 1 FROM reports p WHERE p.recipe=r.id AND p.user=? AND p.active=1)" : " AND r.status='visible' AND author.blocked=0");
      const args = mine || reported ? [...group,user.id] : group;
      const rows = db.prepare('SELECT * FROM ('+selectRecipes+where+') ORDER BY (likes-dislikes) DESC,likes DESC,updated DESC,id LIMIT 25 OFFSET ?').all(...args,offset);
      const count = db.prepare('SELECT COUNT(*) count FROM recipes r JOIN users author ON author.id=r.author'+where).get(...args).count;
      return { items: rows.slice(0,24).map(row=>card(row,game)), total:Number(count), nextOffset:rows.length>24?offset+24:null };
    }
    if (method === 'GET' && route === '/v1/games') {
      if(crossStore)return {items:games.search(url.searchParams.get('q') || '')};
      const q = text(url.searchParams.get('q') || '',120);
      need(q.length>=2,'query_too_short');
      const escaped = q.replace(/[\\%_]/g, c=>'\\'+c);
      const rows = db.prepare(`SELECT r.game id,json_extract(r.content,'$.game.name') name,COUNT(*) recipes
        FROM recipes r JOIN users u ON u.id=r.author WHERE r.status='visible' AND u.blocked=0
        AND json_extract(r.content,'$.game.name') LIKE ? ESCAPE '\\' GROUP BY r.game ORDER BY recipes DESC LIMIT 20`).all('%'+escaped+'%');
      return { items:rows };
    }
    const match = /^\/v1\/recipes\/([a-f0-9-]{36})(?:\/(like|vote|report))?$/.exec(route);
    if (method === 'GET' && match && !match[2]) {
      const row=lookup(match[1]);
      need(row && row.status!=='deleted' && (row.status==='visible' || row.author===user?.id || user?.admin),'not_found',404);
      need(!db.prepare('SELECT blocked FROM users WHERE id=?').get(row.author).blocked || user?.admin,'not_found',404);
      return card(row);
    }
    if (user && method!=='GET') limit(db,'write:'+user.id,t,60,30);
    if (method === 'POST' && route === '/v1/recipes') {
      exact(body,['submission','content']); need(uuid(body.submission),'invalid_submission');
      const content = recipe(body.content), encoded=JSON.stringify(content);
      return transaction(db,()=>{
        const old=db.prepare('SELECT id,content,status FROM recipes WHERE author=? AND submission=?').get(user.id,body.submission);
        if(old) { need(old.content===encoded && old.status!=='deleted','submission_conflict',409); return card(lookup(old.id)); }
        const group=games.ids(content.game.id);
        const count=db.prepare(`SELECT COUNT(*) count FROM recipes WHERE author=? AND game IN (${group.map(()=>'?')}) AND status!='deleted'`).get(user.id,...group).count;
        need(count<5,'five_recipes_per_game',409);
        const id=randomUUID();
        db.prepare('INSERT INTO recipes(id,author,game,content,updated,submission) VALUES(?,?,?,?,?,?)').run(id,user.id,content.game.id,encoded,t,body.submission);
        return card(lookup(id));
      });
    }
    if (match && method!=='GET') return transaction(db,()=>{
      const row=lookup(match[1]); need(row && row.status!=='deleted','not_found',404);
      if (['like','vote'].includes(match[2]) && method==='PUT') {
        if(match[2]==='like') { exact(body,['revision','liked']); need(typeof body.liked==='boolean','invalid_like'); }
        else { exact(body,['revision','value']); need([-1,0,1].includes(body.value),'invalid_vote'); }
        need(row.status==='visible' && !db.prepare('SELECT blocked FROM users WHERE id=?').get(row.author).blocked,'not_found',404);
        need(row.author!==user.id,'own_recipe_like'); need(row.revision===body.revision,'revision_changed',409);
        const value=match[2]==='like'?(body.liked?1:0):body.value;
        db.prepare('DELETE FROM likes WHERE recipe=? AND revision=? AND user=?').run(row.id,row.revision,user.id);
        if(match[2]==='vote' || body.liked) db.prepare('DELETE FROM dislikes WHERE recipe=? AND revision=? AND user=?').run(row.id,row.revision,user.id);
        if(value) db.prepare(`INSERT INTO ${value===1?'likes':'dislikes'} VALUES(?,?,?)`).run(row.id,row.revision,user.id);
        return card(lookup(row.id));
      }
      if (match[2]==='report' && ['PUT','DELETE'].includes(method)) {
        need(row.author!==user.id,'own_recipe_report');
        if(method==='DELETE') {
          exact(body,[]);
          db.prepare('UPDATE reports SET active=0 WHERE recipe=? AND user=?').run(row.id,user.id);
        } else {
          exact(body,['reason']); const reason=text(body.reason,200);
          const old=db.prepare('SELECT * FROM reports WHERE recipe=? AND user=?').get(row.id,user.id);
          need(row.status==='visible' || old?.active,'not_found',404);
          need(!old || old.reviewed_revision<row.revision,'report_reviewed',409);
          db.prepare(`INSERT INTO reports(recipe,user,reason,created) VALUES(?,?,?,?)
            ON CONFLICT(recipe,user) DO UPDATE SET reason=excluded.reason,created=excluded.created,active=1`).run(row.id,user.id,reason,t);
          const count=db.prepare('SELECT COUNT(*) n FROM reports p JOIN users u ON u.id=p.user WHERE p.recipe=? AND p.active=1 AND u.blocked=0 AND p.user!=?').get(row.id,row.author).n;
          if(row.status==='visible' && count>=3) {
            db.prepare("UPDATE recipes SET status='hidden' WHERE id=?").run(row.id);
            db.prepare("INSERT INTO audit(actor,action,target,created) VALUES('system','report_threshold_hide',?,?)").run(row.id,t);
          }
        }
        return ['3','4'].includes(feedback) ? card(lookup(row.id)) : {ok:true};
      }
      need(!match[2],'not_found',404); need(row.author===user.id,'not_owner',403);
      if(method==='DELETE') {
        exact(body,['revision']); need(row.revision===body.revision,'revision_changed',409);
        deleteRecipe(db,row.id,t);
        return { ok:true };
      }
      need(method==='PUT','not_found',404); exact(body,['revision','content']);
      need(row.revision===body.revision,'revision_changed',409);
      const content=recipe(body.content); need(games.resolve(content.game.id)===games.resolve(row.game),'game_cannot_change');
      const original=JSON.parse(row.content);content.game=original.game;
      if(!crossStore)for(const field of ['platform','edition'])if(original[field])content[field]=original[field];
      db.prepare('UPDATE recipes SET content=?,revision=revision+1,updated=? WHERE id=?').run(JSON.stringify(content),t,row.id);
      // Feedback belongs to the shared setup across edits. Keep the revision guard
      // for stale writes, while moving the current votes atomically with the edit.
      for (const table of ['likes','dislikes']) db.prepare(`UPDATE ${table} SET revision=? WHERE recipe=? AND revision=?`).run(row.revision+1,row.id,row.revision);
      return card(lookup(row.id));
    });
    if (route.startsWith('/v1/admin')) {
      need(user?.admin,'admin_required',403);
      if(method==='GET' && route==='/v1/admin/games')return {items:games.search(url.searchParams.get('q') || '',true),...games.history()};
      if(method==='GET' && route==='/v1/admin/game-associations/preview')return games.preview(url.searchParams.get('source'),url.searchParams.get('target'));
      if(method==='POST' && route==='/v1/admin/game-associations')return games.merge(body,user.id,t);
      if(method==='POST' && route==='/v1/admin/game-associations/undo')return games.undo(body,user.id,t);
      if(method==='GET' && route==='/v1/admin/mail-health') return options.mailHealth?.() || {ok:false,state:'not_configured'};
      if(method==='GET' && route==='/v1/admin/recipes') {
        const offset=Number(url.searchParams.get('offset')||0), filter=url.searchParams.get('filter')||'all';
        const query=text(url.searchParams.get('q')||'',120,true);
        need(Number.isSafeInteger(offset) && offset>=0 && offset<=10000,'invalid_page');
        need(['all','visible','hidden','reported','duplicates'].includes(filter),'invalid_filter');
        // Suspected duplicates share the game, patch, sorted loaders, API and GPU requirements.
        // Notes may differ: show candidates for review, never delete them automatically.
        const cte=`WITH candidates AS (${selectRecipes} WHERE r.status!='deleted'), keyed AS (
          SELECT *,json_array(COALESCE((SELECT canonical FROM game_keys WHERE id=c.game),game),json_extract(content,'$.platform'),json_extract(content,'$.edition'),json_extract(content,'$.scheme'),json_extract(content,'$.patchVersion'),
          (SELECT json_group_array(value) FROM (SELECT value FROM json_each(json_extract(c.content,'$.entries')) ORDER BY value)),
          json_extract(content,'$.renderer'),json_extract(content,'$.gpuAlias'),json_extract(content,'$.neuralRuntime'),
          lower(replace(json_extract(content,'$.gpuModel'),' ','')),json_extract(content,'$.driver')) duplicateKey
          FROM candidates c), reviewed AS (SELECT *,COUNT(*) OVER(PARTITION BY duplicateKey) duplicateCount,
          (SELECT COUNT(*) FROM reports p JOIN users u ON u.id=p.user WHERE p.recipe=k.id AND p.active=1 AND u.blocked=0) reports FROM keyed k)`;
        const clauses=[],args=[];
        if(['visible','hidden'].includes(filter)) {clauses.push('status=?');args.push(filter);}
        if(filter==='reported')clauses.push('reports>0');
        if(filter==='duplicates')clauses.push('duplicateCount>1');
        if(query) {clauses.push("(json_extract(content,'$.game.name') LIKE ? ESCAPE '\\' OR json_extract(content,'$.title') LIKE ? ESCAPE '\\' OR game=?)");const pattern='%'+query.replace(/[\\%_]/g,c=>'\\'+c)+'%';args.push(pattern,pattern,query);}
        const where=clauses.length?' WHERE '+clauses.join(' AND '):'';
        const total=Number(db.prepare(cte+' SELECT COUNT(*) n FROM reviewed'+where).get(...args).n);
        const rows=db.prepare(cte+' SELECT * FROM reviewed'+where+' ORDER BY updated DESC,id LIMIT 25 OFFSET ?').all(...args,offset);
        return {items:rows.slice(0,24).map(r=>({...publicRecipe(db,r,user,true),reports:Number(r.reports),duplicateCount:Number(r.duplicateCount),authorBlocked:!!db.prepare('SELECT blocked FROM users WHERE id=?').get(r.author).blocked,reasons:db.prepare('SELECT reason FROM reports p JOIN users u ON u.id=p.user WHERE p.recipe=? AND p.active=1 AND u.blocked=0').all(r.id).map(v=>v.reason)})),total,nextOffset:rows.length>24?offset+24:null};
      }
      if(method==='GET' && route==='/v1/admin/reports') {
        const rows=db.prepare(`SELECT r.*,COUNT(p.user) reports FROM recipes r JOIN reports p ON p.recipe=r.id
          JOIN users u ON u.id=p.user WHERE r.status!='deleted' AND p.active=1 AND u.blocked=0 GROUP BY r.id ORDER BY MAX(p.created) DESC LIMIT 100`).all();
        return { items:rows.map(r=>({...publicRecipe(db,r,user),reports:Number(r.reports),reasons:db.prepare('SELECT reason FROM reports WHERE recipe=? AND active=1').all(r.id).map(v=>v.reason)})) };
      }
      if(method==='POST' && route==='/v1/admin/moderate') {
        exact(body,['id','action','revision']); need(uuid(body.id),'invalid_id');
        need(['hide','restore','delete','block_author','unblock_author'].includes(body.action),'invalid_action');
        return transaction(db,()=>{
          const row=lookup(body.id); need(row && row.status!=='deleted','not_found',404);
          if(body.action==='delete' || body.revision!==undefined)need(body.revision===row.revision,'revision_changed',409);
          if(body.action==='delete') deleteRecipe(db,row.id,t);
          else if(body.action==='block_author' || body.action==='unblock_author') {
            need(row.author!==user.id,'cannot_block_self');
            db.prepare('UPDATE users SET blocked=? WHERE id=?').run(body.action==='block_author'?1:0,row.author);
            db.prepare('DELETE FROM sessions WHERE user=?').run(row.author);
          } else {
            db.prepare('UPDATE recipes SET status=? WHERE id=?').run(body.action==='hide'?'hidden':'visible',row.id);
            if(body.action==='restore') db.prepare('UPDATE reports SET active=0,reviewed_revision=? WHERE recipe=?').run(row.revision,row.id);
          }
          db.prepare('INSERT INTO audit(actor,action,target,created) VALUES(?,?,?,?)').run(user.id,body.action,row.id,t);
          return { ok:true };
        });
      }
    }
    throw new ApiError('not_found',404);
  };
}
