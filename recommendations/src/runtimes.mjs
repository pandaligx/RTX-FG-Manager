import {exact,need} from './contracts.mjs';
const version=v=>typeof v==='string' && (v==='unknown' || v.length<=40 && /^\d{1,10}\.\d{1,10}\.\d{1,10}(?:\.\d{1,10})?$/.test(v));
export function runtimeSnapshot(input){
  exact(input,['schema','components']);
  need(input.schema===1 && Array.isArray(input.components) && input.components.length<=21,'invalid_runtime_snapshot');
  const seen=new Set();
  const components=input.components.map(c=>{
    exact(c,['kind','versions','source','packageVersion']);
    need(['sr','rr','fg','nr','xess','xess_fg','xell'].includes(c.kind)
      && ['game','patch','managed'].includes(c.source),'invalid_runtime_component');
    const key=c.source+':'+c.kind;need(!seen.has(key),'duplicate_runtime_component');seen.add(key);
    need(Array.isArray(c.versions) && c.versions.length>0 && c.versions.length<=8
      && c.versions.every(version) && new Set(c.versions).size===c.versions.length,'invalid_runtime_version');
    need(c.source!=='managed' || ['xess_fg','xell'].includes(c.kind),'invalid_runtime_source');
    need(c.packageVersion===undefined || c.source==='managed' && version(c.packageVersion) && c.packageVersion!=='unknown','invalid_runtime_package');
    return {kind:c.kind,source:c.source,versions:[...c.versions].sort(),...(c.packageVersion===undefined?{}:{packageVersion:c.packageVersion})};
  }).sort((a,b)=>(a.source+':'+a.kind).localeCompare(b.source+':'+b.kind,'en'));
  return {schema:1,components};
}
