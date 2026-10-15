'use strict';
const params = new URLSearchParams(location.search);
const supported = ['zh','en','ru','ja','ko'];
const sectionIds = ['start','patches','menus','community','maintenance','faq'];
const $ = id => document.getElementById(id);
const make = (tag,text,cls) => {const e=document.createElement(tag);if(text!=null)e.textContent=text;if(cls)e.className=cls;return e;};
const safeStore = {get(k){try{return localStorage.getItem(k);}catch{return null;}},set(k,v){try{localStorage.setItem(k,v);}catch{}}};
let language = supported.includes(params.get('lang')) ? params.get('lang') : 'zh';
let selectedPatch = 'dlssg', toastTimer, observer;
let theme = ['light','dark'].includes(params.get('theme')) ? params.get('theme') : safeStore.get('help-theme') || 'system';
const content = () => HELP_CONTENT[language] || HELP_CONTENT.en;
const patchMeta = () => PATCH_META[language] || PATCH_META.en;
const completed = new Set();
function applyTheme(){document.documentElement.dataset.theme=theme==='system'?(matchMedia('(prefers-color-scheme:dark)').matches?'dark':'light'):theme;}
applyTheme();
matchMedia('(prefers-color-scheme:dark)').addEventListener('change',applyTheme);
$('theme').addEventListener('click',()=>{theme=document.documentElement.dataset.theme==='dark'?'light':'dark';safeStore.set('help-theme',theme);applyTheme();});
async function copy(text){
  try{await navigator.clipboard.writeText(text);}catch{
    const input=make('textarea');input.value=text;document.body.append(input);input.select();document.execCommand('copy');input.remove();
  }
  $('toast').textContent=content().copied;$('toast').classList.add('show');clearTimeout(toastTimer);
  toastTimer=setTimeout(()=>$('toast').classList.remove('show'),1800);
}
function section(id,title){const e=make('section',null,'section');e.id=id;const head=make('div',null,'section-head');head.append(make('span',String(sectionIds.indexOf(id)+1).padStart(2,'0')),make('h2',title));e.append(head);return e;}
function list(items){const e=make('ul');items.forEach(text=>e.append(make('li',text)));return e;}
function details(title,text){const e=make('details');e.append(make('summary',title),make('p',text));return e;}
function checklist(items){
  const e=make('ol',null,'checklist');
  items.forEach((text,i)=>{const row=make('li'),b=make('button',null,'step-button');b.type='button';b.setAttribute('aria-pressed',String(completed.has(i)));b.append(make('span',text));b.addEventListener('click',()=>{if(completed.has(i))completed.delete(i);else completed.add(i);b.setAttribute('aria-pressed',String(completed.has(i)));});row.append(b);e.append(row);});return e;
}
function patchSearch(key){const p=content().patches[key];return [p.name,...p.steps,p.note,p.advanced,...p.copy,...patchMeta()[key]].join(' ').toLocaleLowerCase();}
function renderPatch(){
  const c=content(),meta=patchMeta();
  for(const key of PATCH_KEYS){const button=$('tab-'+key);button.setAttribute('aria-selected',String(key===selectedPatch));button.tabIndex=key===selectedPatch?0:-1;}
  const p=c.patches[selectedPatch],m=meta[selectedPatch],panel=make('div',null,'patch-panel');panel.id='patch-panel';panel.setAttribute('role','tabpanel');panel.setAttribute('aria-labelledby','tab-'+selectedPatch);panel.tabIndex=0;
  const head=make('div',null,'patch-panel-top');head.append(make('h3',p.name),make('span',m[4]?c.statusNew:c.statusClassic));
  const facts=make('div',null,'patch-facts');facts.append(make('span',m[2]),make('span',m[3]));
  panel.append(head,facts,list(p.steps),make('p',p.note,'note'),details(c.advanced,p.advanced));
  const chips=make('div',null,'chips');for(const text of p.copy){const b=make('button',text+'  ⧉','copy');b.type='button';b.setAttribute('aria-label',c.copy+' '+text);b.addEventListener('click',()=>copy(text));chips.append(b);}panel.append(chips);
  const previous=$('patch-panel');if(previous)previous.replaceWith(panel);else $('patches').append(panel);
}
function choosePatch(key,focus=false){selectedPatch=key;renderPatch();if(focus)$('tab-'+key).focus();}
function render(){
  const c=content();observer?.disconnect();
  document.documentElement.lang={zh:'zh-CN',en:'en',ru:'ru',ja:'ja',ko:'ko'}[language];
  document.title=c.contents+' · RTX FG Manager';$('language').value=language;$('title').textContent=c.title;$('intro').textContent=c.intro;
  $('eyebrow').textContent='THE FIELD GUIDE / 06 PATCHES';$('edition').textContent='GUIDE · '+(/^\d+\.\d+\.\d+$/.test(params.get('version'))?'v'+params.get('version'):'v4.3.0');
  $('nav-title').textContent=c.contents;$('offline').textContent=c.offline;$('offline-badge').textContent=c.badge;$('begin').textContent=c.begin;$('explore').textContent=c.explore;
  $('search').placeholder=c.search;$('search').setAttribute('aria-label',c.search);$('theme').setAttribute('aria-label',c.theme);$('top').textContent=c.top;
  $('content-version').textContent=c.version+' 4.3.0.5';$('empty').textContent=c.empty;$('fallback').hidden=!c.fallback;$('fallback').textContent=c.fallback||'';
  $('navigation').replaceChildren();$('sections').replaceChildren();$('shortcuts').replaceChildren();
  for(const [name,key] of [['Encore','Insert'],['MFG Unlocker','Backspace'],['Vulkan MFG','Ctrl+F2—F6']]){const card=make('a',null,'shortcut');card.href='#menus';card.append(make('small',name),make('strong',key));$('shortcuts').append(card);}
  c.sections.forEach((name,i)=>{const a=make('a');a.href='#'+sectionIds[i];a.append(make('span',String(i+1).padStart(2,'0')),make('b',name));$('navigation').append(a);});
  const start=section('start',c.sections[0]);start.append(make('p',c.complete,'section-intro'),checklist(c.start),make('p',c.startNote,'note'));$('sections').append(start);
  const patches=section('patches',c.sections[1]),tabs=make('div',null,'tabs');tabs.setAttribute('role','tablist');tabs.setAttribute('aria-label',c.sections[1]);
  PATCH_KEYS.forEach((key,i)=>{
    const m=patchMeta()[key],button=make('button',null,'patch-card');button.type='button';button.id='tab-'+key;button.setAttribute('role','tab');button.setAttribute('aria-controls','patch-panel');
    const number=make('span',null,'patch-index');number.append(make('span',String(i+1).padStart(2,'0')),make('b','↗'));
    button.append(number,make('strong',m[0]),make('small',m[1]),make('span',m[4]?c.statusNew:c.statusClassic,'patch-tag'));
    button.addEventListener('click',()=>choosePatch(key));
    button.addEventListener('keydown',event=>{const keys=PATCH_KEYS.filter(k=>!$('tab-'+k).hidden),index=keys.indexOf(key);if(['ArrowLeft','ArrowRight','Home','End'].includes(event.key)){event.preventDefault();const next=event.key==='Home'?keys[0]:event.key==='End'?keys.at(-1):keys[(index+(event.key==='ArrowRight'?1:keys.length-1))%keys.length];choosePatch(next,true);}});
    tabs.append(button);
  });
  patches.append(make('p',c.patchIntro,'section-intro'),tabs);$('sections').append(patches);renderPatch();
  for(const [id,texts] of [['menus',c.menus],['community',c.community],['maintenance',c.maintenance]]){const s=section(id,c.sections[sectionIds.indexOf(id)]);s.append(list(texts));$('sections').append(s);}
  const faq=section('faq',c.sections[5]);for(const [q,a] of c.faq)faq.append(details(q,a));$('sections').append(faq);
  filter();observeSections();updateProgress();
}
function filter(){
  const q=$('search').value.trim().toLocaleLowerCase();let count=0;
  const matches=PATCH_KEYS.filter(key=>!q||patchSearch(key).includes(q));
  if(matches.length&&!matches.includes(selectedPatch))choosePatch(matches[0]);
  for(const key of PATCH_KEYS)$('tab-'+key).hidden=!matches.includes(key);
  for(const section of $('sections').children){const show=!q||(section.id==='patches'?matches.length>0:section.textContent.toLocaleLowerCase().includes(q));section.hidden=!show;if(show){count++;for(const d of section.querySelectorAll('details'))d.open=!!q;}}
  $('empty').hidden=count>0;$('search-count').textContent=q?count+' '+content().results:'';updateProgress();
}
function observeSections(){
  if(!('IntersectionObserver' in window))return;
  observer=new IntersectionObserver(entries=>{for(const entry of entries){if(!entry.isIntersecting)continue;for(const link of $('navigation').children){const active=link.hash==='#'+entry.target.id;link.classList.toggle('active',active);if(active)link.setAttribute('aria-current','location');else link.removeAttribute('aria-current');}for(const section of $('sections').children)section.classList.toggle('is-current',section===entry.target);}},{rootMargin:'-12% 0px -55% 0px',threshold:0});
  for(const section of $('sections').children)observer.observe(section);
}
let scrollScheduled=false;
function updateProgress(){const d=document.documentElement,total=d.scrollHeight-d.clientHeight;$('reading-progress').value=total>0?Math.min(100,100*d.scrollTop/total):100;}
addEventListener('scroll',()=>{if(!scrollScheduled){scrollScheduled=true;requestAnimationFrame(()=>{updateProgress();scrollScheduled=false;});}},{passive:true});
addEventListener('resize',updateProgress);
$('search').addEventListener('input',filter);$('language').addEventListener('change',()=>{language=$('language').value;render();});
document.addEventListener('keydown',event=>{if((event.ctrlKey||event.metaKey)&&event.key.toLowerCase()==='k'){event.preventDefault();$('search').focus();}if(event.key==='Escape'&&document.activeElement===$('search')){$('search').value='';filter();}});
render();const topic=params.get('topic');if(sectionIds.includes(topic))$(topic).scrollIntoView();
