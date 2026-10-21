const $=id=>document.getElementById(id);
let token='',challenge='',offset=0,nextOffset=null,busy=false;
let gameSource=null,gameTarget=null,gamePlan=null,gameRevision=0;
async function mailStatus(){
  try{const response=await fetch('/v1/health/mail',{cache:'no-store'});const data=await response.json();$('mail-status').textContent=response.ok && data.ok?'验证码服务正常 · SMTP 每 5 分钟检查':'验证码服务异常 · 请检查邮箱授权码、发送限制或网络';}
  catch{$('mail-status').textContent='验证码服务暂时无法连接';}
}
void mailStatus();setInterval(()=>{void mailStatus();},60000);
const errors={admin_required:'此邮箱没有管理员权限',session_expired:'登录已过期，请重新登录',revision_changed:'方案已被修改，请刷新后重新确认',not_found:'方案已被删除，请刷新',invalid_or_expired_code:'验证码错误或已过期',invalid_email:'请输入有效邮箱',rate_limited:'操作较频繁，请稍后重试',wait_before_resending:'请稍后再获取验证码',mail_unavailable:'邮件暂时无法发送，请稍后重试',cannot_block_self:'不能限制当前管理员'};
Object.assign(errors,{mapping_changed:'游戏归类已变化，请重新搜索并预览',games_already_linked:'这两个条目已经属于同一游戏',game_not_found:'游戏条目不存在，请重新搜索',mapping_not_reversible:'只能撤销最近一次且未发生其他归类变动的关联'});
function address(){const value=$('email').value.trim();return value && !value.includes('@') && $('domain').value?value+'@'+$('domain').value:value;}
function previewEmail(){$('email-preview').textContent=address();}
$('email').oninput=previewEmail;
$('email').onblur=()=>{$('email').value=address();previewEmail();};
$('domain').onchange=()=>{const value=$('email').value.trim();if($('domain').value && value.includes('@'))$('email').value=value.split('@')[0];previewEmail();};
async function api(path,method='GET',body){
  const response=await fetch('/v1/'+path,{method,headers:{'X-RTXFG-Feedback':'4',...(token?{Authorization:'Bearer '+token}:{}),...(body?{'Content-Type':'application/json'}:{})},body:body?JSON.stringify(body):undefined,cache:'no-store'});
  const value=await response.json();if(!response.ok)throw new Error(errors[value.error]||'操作失败，请稍后重试');return value;
}
async function run(action){if(busy)return;busy=true;$('status').textContent='处理中…';$('review').setAttribute('aria-busy','true');try{await action();$('status').textContent='';}catch(error){$('status').textContent=error.message;}finally{busy=false;$('review').setAttribute('aria-busy','false');}}
$('send').onclick=()=>run(async()=>{$('email').value=address();previewEmail();$('send').disabled=true;try{const r=await api('auth/code','POST',{email:address()});challenge=r.challenge;$('send').textContent='已发送，60 秒后可重发';setTimeout(()=>{$('send').disabled=false;$('send').textContent='发送验证码';},60000);}catch(e){$('send').disabled=false;throw e;}});
$('login').onsubmit=e=>{e.preventDefault();run(async()=>{const r=await api('auth/verify','POST',{challenge,code:$('code').value});token=r.token;$('code').value='';if(!r.user.admin){await api('auth/logout','POST',{});token='';throw new Error('此邮箱没有管理员权限');}$('login').hidden=true;$('review').hidden=false;offset=0;await refresh();});};
function element(tag,text,cls){const node=document.createElement(tag);node.textContent=text;if(cls)node.className=cls;return node;}
async function refresh(){
  const params=new URLSearchParams({q:$('query').value.trim(),filter:$('filter').value,offset:String(offset)});
  const r=await api('admin/recipes?'+params);nextOffset=r.nextOffset;$('cards').replaceChildren();
  $('count').textContent='共 '+r.total+' 个方案 · 第 '+(Math.floor(offset/24)+1)+' 页';
  $('previous').disabled=offset===0;$('next').disabled=nextOffset===null;
  for(const item of r.items){
    const c=item.content,card=document.createElement('article'),head=element('div','','card-heading');
    head.append(element('h2',c.title),element('span',item.status==='hidden'?'已隐藏':'公开','badge'));
    card.append(head,element('p',c.game.name+' · '+c.patchVersion+' · '+c.entries.join(' + '),'muted'),element('p','有用 '+item.likes+' · 无用 '+item.dislikes+' · 举报 '+item.reports+(item.authorBlocked?' · 作者已限制':''),'meta'));
    if(item.duplicateCount>1)card.append(element('p','疑似重复：'+item.duplicateCount+' 个方案使用相同游戏、补丁和硬件条件，请展开比较备注。','notice'));
    if(c.notes)card.append(element('p',c.notes));
    if(item.reports)card.append(element('p','同一邮箱计一次，可撤销；3 个不同已验证邮箱举报后暂时隐藏。恢复公开会结案当前举报，旧举报不能重复隐藏同一版本。','notice'));
    const details=document.createElement('details');details.append(element('summary','查看完整方案与举报'),element('pre',JSON.stringify({游戏标识:c.game.id,作者标识:item.author,方案:c,举报:item.reasons},null,2)));card.append(details);
    const footer=document.createElement('footer');
    const actions=[[item.status==='hidden'?'restore':'hide',item.status==='hidden'?'恢复公开':'隐藏方案'],[item.authorBlocked?'unblock_author':'block_author',item.authorBlocked?'解除作者限制':'限制作者'],['delete','删除方案']];
    for(const[action,label]of actions){const b=element('button',label,action==='delete'?'danger':'');b.onclick=()=>run(async()=>{const consequence=action==='delete'?'将删除该方案正文、评价和举报，无法在这里恢复。':action==='block_author'?'该作者的所有方案将不再公开，且无法发布或评价。':action==='restore'?'恢复公开并结案当前举报，保留有用/无用评价。':'此操作只改变方案或作者的可用状态。';if(!confirm(label+'「'+c.title+'」？\n'+c.game.name+'\n'+consequence))return;await api('admin/moderate','POST',{id:item.id,action,revision:item.revision});await refresh();});footer.append(b);}card.append(footer);$('cards').append(card);
  }
  if(!r.items.length)$('cards').append(element('p','没有符合条件的方案。','empty'));
}
$('search').onsubmit=e=>{e.preventDefault();run(async()=>{offset=0;await refresh();});};
$('filter').onchange=()=>run(async()=>{offset=0;await refresh();});
$('refresh').onclick=()=>run(refresh);
$('previous').onclick=()=>run(async()=>{offset=Math.max(0,offset-24);await refresh();});
$('next').onclick=()=>run(async()=>{if(nextOffset!==null){offset=nextOffset;await refresh();}});
$('logout').onclick=()=>run(async()=>{await api('auth/logout','POST',{});token='';$('review').hidden=true;$('login').hidden=false;$('cards').replaceChildren();gameSource=gameTarget=gamePlan=null;$('game-results').replaceChildren();gameSelection();});
function gameSelection(){
  gamePlan=null;$('game-plan').hidden=true;
  $('game-selection').textContent='待归类：'+(gameSource?.name||'未选择')+' → 正确游戏：'+(gameTarget?.name||'未选择');
  $('game-preview').disabled=!gameSource || !gameTarget || gameSource.id===gameTarget.id;
}
async function loadGames(){
  const r=await api('admin/games?'+new URLSearchParams({q:$('game-query').value.trim()}));gameRevision=r.revision;
  $('game-results').replaceChildren();gameSelection();
  for(const item of r.items){
    const row=element('article',''),head=element('h3',item.name);row.append(head,element('p',[item.original,item.recipes+' 个分享'].filter(Boolean).join(' · '),'meta'),element('p',item.keys.join(' · '),'meta'));
    const buttons=element('nav','');
    for(const [label,source] of [['设为待归类',true],['设为正确游戏',false]]){
      const button=element('button',label);button.onclick=()=>{if(busy)return;if(source)gameSource=item;else gameTarget=item;gameSelection();};buttons.append(button);
    }
    row.append(buttons);$('game-results').append(row);
  }
  if(!r.items.length)$('game-results').append(element('p','没有找到。可尝试另一种语言的名称，或先由玩家分享该游戏。','muted'));
  const last=r.history[0];$('game-history').textContent=last?'最近关联：'+last.source+' → '+last.target+(last.undone?'（已撤销）':''):'还没有人工归类记录';$('game-undo').disabled=!last || !!last.undone;
}
$('game-directory').ontoggle=()=>{if($('game-directory').open && token)run(loadGames);};
$('game-search').onsubmit=e=>{e.preventDefault();run(loadGames);};
$('game-preview').onclick=()=>run(async()=>{if(!gameSource || !gameTarget)return;gamePlan=await api('admin/game-associations/preview?'+new URLSearchParams({source:gameSource.id,target:gameTarget.id}));$('game-plan-text').textContent=`将「${gamePlan.source.name}」的 ${gamePlan.source.recipes} 个分享归入「${gamePlan.target.name}」（已有 ${gamePlan.target.recipes} 个）。新客户端刷新后可共同浏览；不会删除方案或清空评价。`;$('game-plan').hidden=false;});
$('game-merge').onclick=()=>run(async()=>{if(!gamePlan)return;await api('admin/game-associations','POST',{source:gamePlan.source.id,target:gamePlan.target.id,revision:gamePlan.revision});gameSource=gameTarget=null;await loadGames();await refresh();});
$('game-undo').onclick=()=>run(async()=>{if(!confirm('撤销最近一次关联？方案和评价会保留，游戏分组恢复到关联之前。'))return;await api('admin/game-associations/undo','POST',{revision:gameRevision});gameSource=gameTarget=null;await loadGames();await refresh();});

