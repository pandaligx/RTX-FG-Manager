const $=id=>document.getElementById(id);
let token='',challenge='',offset=0,nextOffset=null,busy=false;
let items=[],selectedId=null,metadata={},activeView='recipes';
$('today').textContent=new Intl.DateTimeFormat('zh-CN').format(new Date());
let gameSource=null,gameTarget=null,gamePlan=null,gameRevision=0;
async function mailStatus(){
  try{const response=await fetch('/v1/health/mail',{cache:'no-store',signal:AbortSignal.timeout(20000)});const data=await response.json();$('mail-status').dataset.state=response.ok && data.ok?'ok':'error';$('mail-status').textContent=response.ok && data.ok?'验证码服务正常':'验证码服务异常，请检查邮箱配置';$('mail-status').title='SMTP 每 5 分钟检查，不发送邮件';}
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
  const response=await fetch('/v1/'+path,{method,headers:{'X-RTXFG-Feedback':'9',...(token?{Authorization:'Bearer '+token}:{}),...(body?{'Content-Type':'application/json'}:{})},body:body?JSON.stringify(body):undefined,cache:'no-store'});
  const value=await response.json();if(!response.ok){if(response.status===401){token='';$('review').hidden=true;$('login').hidden=false;$('logout').hidden=true;}throw new Error(errors[value.error]||'操作失败，请稍后重试');}return value;
}
async function run(action){if(busy)return;busy=true;$('status').textContent='处理中…';$('review').setAttribute('aria-busy','true');try{await action();$('status').textContent='';}catch(error){$('status').textContent=error.message;}finally{busy=false;$('review').setAttribute('aria-busy','false');}}
$('send').onclick=()=>run(async()=>{$('email').value=address();previewEmail();$('send').disabled=true;try{const r=await api('auth/code','POST',{email:address()});challenge=r.challenge;$('send').textContent='已发送，60 秒后可重发';setTimeout(()=>{$('send').disabled=false;$('send').textContent='发送验证码';},60000);}catch(e){$('send').disabled=false;throw e;}});
$('login').onsubmit=e=>{e.preventDefault();run(async()=>{const r=await api('auth/verify','POST',{challenge,code:$('code').value});token=r.token;$('code').value='';if(!r.user.admin){await api('auth/logout','POST',{});token='';throw new Error('此邮箱没有管理员权限');}$('login').hidden=true;$('review').hidden=false;$('logout').hidden=false;offset=0;await refresh();});};
function element(tag,text,cls){const node=document.createElement(tag);node.textContent=text;if(cls)node.className=cls;return node;}
function ask(message){return new Promise(resolve=>{
  const dialog=element('dialog','','confirm-dialog'),title=element('h2','确认审核操作'),copy=element('p',message),actions=element('div','','confirm-actions');
  title.id='confirm-title';dialog.setAttribute('aria-labelledby',title.id);
  const cancel=element('button','取消'),ok=element('button','确认','primary');cancel.autofocus=true;
  cancel.onclick=()=>dialog.close('cancel');ok.onclick=()=>dialog.close('confirm');
  dialog.onclose=()=>{const accepted=dialog.returnValue==='confirm';dialog.remove();resolve(accepted);};
  actions.append(cancel,ok);dialog.append(title,copy,actions);document.body.append(dialog);dialog.showModal();
});}
function schemeName(id){return {'dlssg-transfusion-1.4.5.3':'RTX Encore','rtx40mfg-1.3.3-hf2':'MFG Unlocker','upstream-0.3.5-310-9':'Dlssg_for_sm86','rtxfg-0.3.5-dx12-vulkan':'Dlssg_for_sm86 三角洲','pipotoufik-mfg-vulkan-sm86-7':'MFG Vulkan'}[id]||id;}
function dateLabel(item){const value=item.updatedAt||item.createdAt;if(!value)return '';const date=new Date(typeof value==='number'?value*1000:value);return Number.isNaN(date.getTime())?'':new Intl.DateTimeFormat('zh-CN',{year:'numeric',month:'2-digit',day:'2-digit'}).format(date);}

function icon(name,cls=''){const img=document.createElement('img');img.src='/icons/'+name+'.svg';img.alt='';if(cls)img.className=cls;return img;}
function gameIcon(){const box=element('span','','game-icon');box.append(icon('gamepad'));return box;}
function badge(item){return element('span',item.status==='hidden'?'已隐藏':'公开','badge '+item.status);}
function stat(name,label){const span=element('span','','stat');span.append(icon(name),document.createTextNode(label));return span;}
function section(title,name='sliders-horizontal',hint=''){const s=element('section','','detail-section'),head=element('div','','section-heading');head.append(icon(name),element('h3',title));if(hint)head.append(element('span',hint,'muted'));s.append(head);return s;}
function presetDetails(snapshot,scheme){
  const s=section('可视化参数对比','sliders-horizontal','仅显示修改项');
  if(!snapshot){s.append(element('p','此分享未附带预设参数。','muted'));return s;}
  const changed=Object.entries(snapshot.values).filter(([k,v])=>v!==snapshot.defaults[k] && !(v.trim() && snapshot.defaults[k]?.trim() && Number.isFinite(Number(v)) && Number(v)===Number(snapshot.defaults[k])));
  if(!changed.length){s.append(element('p','使用默认参数。','muted'));return s;}
  const table=element('div','','visual-comparison'),head=element('div','','comparison-head');
  for(const text of ['参数','默认值','','玩家设置'])head.append(element('span',text));table.append(head);
  for(const [key,value] of changed){
    const field=(snapshot.profile==='upstream041'?metadata[scheme]?.from_041:metadata[scheme])?.fields?.[key]||{},before=snapshot.defaults[key],row=element('div','','comparison-row');
    row.append(element('span',field.label||key,'comparison-label'));
    const numeric=field.number && Number.isFinite(field.min) && Number.isFinite(field.max) && field.min<field.max
      && [before,value].every(v=>v.trim() && Number.isFinite(Number(v)) && Number(v)>=field.min && Number(v)<=field.max);
    for(const [side,v] of [['default',before],['player',value]]){
      if(side==='player')row.append(icon('arrow-right','comparison-arrow'));
      const cell=element('div','','comparison-value '+side),label=field.labels?.[v]||v;
      if(numeric){
        cell.append(element('span',label,'meter-value'));
        const meter=document.createElement('meter');meter.min=field.min;meter.max=field.max;meter.value=Number(v);
        meter.setAttribute('aria-label',(field.label||key)+(side==='player'?' 玩家设置':' 默认值'));cell.append(meter);
        const axis=element('span','','axis');axis.append(element('span',String(field.min)),element('span',String(field.max)));cell.append(axis);
      }else cell.append(element('span',label,'value-chip'));
      row.append(cell);
    }table.append(row);
  }s.append(table);return s;
}
function runtimeDetails(snapshot){
  const s=section('配套运行库','cpu');
  if(!snapshot){s.append(element('p','此分享未提供运行库版本。','muted'));return s;}
  if(!snapshot.components.length){s.append(element('p','未检测到可共享的运行库。','muted'));return s;}
  const wrap=element('div','','table-wrap'),table=element('table',''),head=element('thead',''),row=element('tr',''),body=element('tbody','');
  for(const label of ['组件','版本号','来源']){const th=element('th',label);th.scope='col';row.append(th);}head.append(row);
  const names={sr:'DLSS SR',rr:'DLSS RR',fg:'DLSS FG',nr:'DLSSNR',xess:'XeSS SR',xess_fg:'XeSS FG',xell:'XeLL'};
  for(const c of snapshot.components){
    const tr=element('tr',''),name=element('td',''),label=element('span','','runtime-name');label.append(icon('check'),document.createTextNode(names[c.kind]||c.kind));name.append(label);
    const version=element('td',c.versions.map(v=>v==='unknown'?'版本未知':v).join(' / '),'runtime-version');
    if(c.packageVersion)version.append(element('div','SDK '+c.packageVersion,'muted'));
    tr.append(name,version,element('td',({game:'游戏目录',patch:'随补丁提供',managed:'管理器安装'})[c.source]||c.source));body.append(tr);
  }
  table.append(head,body);wrap.append(table);s.append(wrap);return s;
}
function selectItem(id,open=false){
  selectedId=id;for(const row of $('cards').children){const active=row.dataset.id===id;row.classList.toggle('selected',active);row.setAttribute('aria-pressed',String(active));}
  const item=items.find(x=>x.id===id);if(item)renderDetail(item);else $('detail').replaceChildren(element('p','没有符合条件的方案。','empty'));
  if(open)$('recipe-workspace').classList.add('show-detail');
}
function moderateButton(item,action,label,cls=''){
  const b=element('button',label,cls);b.onclick=()=>run(async()=>{
    const consequence=action==='delete'?'删除正文、评价和举报，无法恢复。':action==='block_author'?'该作者的所有方案将不再公开，且无法发布或评价。':action==='restore'?'恢复公开并结案当前举报，保留评价。':'改变方案或作者的可用状态。';
    if(!await ask(label+'「'+item.content.title+'」？\n'+item.content.game.name+'\n'+consequence))return;
    await api('admin/moderate','POST',{id:item.id,action,revision:item.revision});await refresh();
  });return b;
}
function renderDetail(item){
  const c=item.content,scroll=element('div','','detail-scroll'),back=element('div','','mobile-back'),backButton=element('button','');
  backButton.setAttribute('aria-label','返回方案列表');backButton.append(icon('arrow-left'));backButton.onclick=()=>{$('recipe-workspace').classList.remove('show-detail');$('cards').querySelector('.selected')?.focus();};back.append(backButton,element('strong','方案管理'));
  const heading=element('div','','detail-heading'),headingText=element('div','','heading-text'),title=element('div','','heading-title');
  title.append(element('h2',c.game.name),badge(item));headingText.append(title,element('p',c.title,'detail-subtitle'));
  const meta=element('div','','detail-meta');meta.append(stat('user',item.authorLabel||'玩家 '+item.author.slice(0,6)),element('span',dateLabel(item)),stat('thumbs-up','有用 '+item.likes),stat('thumbs-down','无用 '+item.dislikes),stat('triangle-alert','举报 '+item.reports));
  headingText.append(meta);heading.append(gameIcon(),headingText);scroll.append(heading);
  if(item.duplicateCount>1)scroll.append(element('p','发现 '+item.duplicateCount+' 个相近方案，请核对参数与备注。','notice'));
  if(item.authorBlocked)scroll.append(element('p','作者已受限，其分享不会公开。','notice'));
  const info=section('基础信息','sliders-horizontal'),grid=element('dl','','detail-grid');
  const aliases={unchanged:'无需修改',rtx4060:'RTX 4060',rtx5090:'RTX 5090',unknown:'未说明'};
  const platforms={steam:'Steam',epic:'Epic',xbox:'Xbox / Game Pass',wegame:'WeGame',gog:'GOG',ea:'EA',ubisoft:'Ubisoft',battlenet:'战网',unknown:'未说明'};
  for(const [k,v] of [['补丁方案',schemeName(c.scheme)+' '+c.patchVersion],['游戏平台',platforms[c.platform]||c.platform||'未说明'],['DLL 入口',c.entries.join(' + ')],['玩家显卡',c.gpuModel||'未说明'],['图形接口',c.renderer?.toUpperCase()||'未说明'],['驱动版本',c.driver||'未说明'],['显卡修改',aliases[c.gpuAlias]||c.gpuAlias],['游戏版本',c.edition||'未说明']]){
    const datum=element('div','','datum');datum.append(element('dt',k),element('dd',v));grid.append(datum);
  }info.append(grid);scroll.append(info,presetDetails(c.presets,c.scheme),runtimeDetails(c.runtimes));
  const notes=section('玩家备注','list');notes.append(element('p',c.notes||'没有填写备注。','notes'));scroll.append(notes);
  if(item.reasons?.length){const reports=section('玩家举报','shield'),list=element('ul','','report-list');for(const r of item.reasons)list.append(element('li',r));reports.append(list);scroll.append(reports);}
  const raw=element('details','','raw-data');raw.append(element('summary','查看原始数据'),element('pre',JSON.stringify({id:item.id,author:item.author,revision:item.revision,...c},null,2)));scroll.append(raw);
  const footer=element('footer','','detail-footer');
  footer.append(moderateButton(item,'restore',item.status==='hidden'?'恢复公开':'保留公开','primary'));
  if(item.status!=='hidden')footer.append(moderateButton(item,'hide','隐藏方案'));
  const more=element('details','','more-actions'),summary=element('summary',''),menu=element('div','','more-menu');summary.setAttribute('aria-label','更多审核操作');summary.append(icon('ellipsis'));menu.append(moderateButton(item,item.authorBlocked?'unblock_author':'block_author',item.authorBlocked?'解除作者限制':'限制作者'),moderateButton(item,'delete','删除方案','danger'));more.append(summary,menu);footer.append(more);
  $('detail').replaceChildren(back,scroll,footer);
}
async function refresh(){
  const params=new URLSearchParams({q:$('query').value.trim(),filter:$('filter').value,offset:String(offset)});
  const r=await api('admin/recipes?'+params);nextOffset=r.nextOffset;items=r.items;
  // Metadata is optional for browsing, and may be retried on the next refresh.
  if(!Object.keys(metadata).length){try{metadata=await api('preset-metadata');}catch{metadata={};}}
  $('count').textContent='共 '+r.total+' 个方案';$('previous').disabled=offset===0;$('next').disabled=nextOffset===null;$('cards').replaceChildren();
  for(const item of items){
    const c=item.content,row=element('button','','recipe-row'),text=element('span','','row-text'),title=element('span','','row-title'),meta=element('span','','row-meta');row.dataset.id=item.id;
    title.append(element('strong',c.game.name),badge(item));meta.append(stat('user',item.authorLabel||'玩家 '+item.author.slice(0,6)),element('span',dateLabel(item)));
    text.append(title,element('span',c.title,'row-subtitle'),meta);row.append(gameIcon(),text);row.onclick=()=>{if(!busy)selectItem(item.id,true);};$('cards').append(row);
  }
  if(!items.length){$('cards').append(element('p','没有符合条件的方案。','empty'));$('recipe-workspace').classList.remove('show-detail');}
  selectItem(items.some(v=>v.id===selectedId)?selectedId:items[0]?.id);
}
function setView(view){
  if(busy || !token)return;
  activeView=view;$('game-directory').hidden=view!=='games';$('recipe-workspace').hidden=view==='games';$('recipe-workspace').classList.remove('show-detail');
  for(const name of ['recipes','games','reports']){const active=name===view;$('nav-'+name).classList.toggle('active',active);if(active)$('nav-'+name).setAttribute('aria-current','page');else $('nav-'+name).removeAttribute('aria-current');}
  $('page-title').textContent=({recipes:'推荐方案管理',games:'游戏名称与归类',reports:'举报处理'})[view];
  if(view==='games')run(loadGames);else {$('filter').value=view==='reports'?'reported':'all';offset=0;run(refresh);}
}

$('search').onsubmit=e=>{e.preventDefault();run(async()=>{offset=0;await refresh();});};
$('filter').onchange=()=>run(async()=>{offset=0;await refresh();});
$('refresh').onclick=()=>{if(token)run(activeView==='games'?loadGames:refresh);else void mailStatus();};
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
for(const view of ['recipes','games','reports'])$('nav-'+view).onclick=()=>setView(view);
$('game-search').onsubmit=e=>{e.preventDefault();run(loadGames);};
$('game-preview').onclick=()=>run(async()=>{if(!gameSource || !gameTarget)return;gamePlan=await api('admin/game-associations/preview?'+new URLSearchParams({source:gameSource.id,target:gameTarget.id}));$('game-plan-text').textContent=`将「${gamePlan.source.name}」的 ${gamePlan.source.recipes} 个分享归入「${gamePlan.target.name}」（已有 ${gamePlan.target.recipes} 个）。新客户端刷新后可共同浏览；不会删除方案或清空评价。`;$('game-plan').hidden=false;});
$('game-merge').onclick=()=>run(async()=>{if(!gamePlan)return;await api('admin/game-associations','POST',{source:gamePlan.source.id,target:gamePlan.target.id,revision:gamePlan.revision});gameSource=gameTarget=null;await loadGames();await refresh();});
$('game-undo').onclick=()=>run(async()=>{if(!await ask('撤销最近一次关联？方案和评价会保留，游戏分组恢复到关联之前。'))return;await api('admin/game-associations/undo','POST',{revision:gameRevision});gameSource=gameTarget=null;await loadGames();await refresh();});

