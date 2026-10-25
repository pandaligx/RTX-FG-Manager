import {readFileSync} from 'node:fs';
import {exact,need} from './contracts.mjs';
const contracts=JSON.parse(readFileSync(new URL('./preset-contracts.json',import.meta.url),'utf8'));
export const presetMetadata=()=>contracts;
function compare(a,b){
  const parse=s=>{const [core,pre]=s.split('+',1)[0].split('-',2);return [core.split('.').map(Number),pre];};
  const [av,ap]=parse(a),[bv,bp]=parse(b);
  for(let i=0;i<3;i++)if(av[i]!==bv[i])return av[i]-bv[i];
  if(ap===bp)return 0;if(!ap)return 1;if(!bp)return -1;
  const aa=ap.split('.'),bb=bp.split('.');
  for(let i=0;i<Math.max(aa.length,bb.length);i++){
    if(aa[i]===undefined)return -1;if(bb[i]===undefined)return 1;
    if(aa[i]===bb[i])continue;
    const an=/^\d+$/.test(aa[i]),bn=/^\d+$/.test(bb[i]);
    if(an&&bn)return Number(aa[i])-Number(bb[i]);if(an!==bn)return an?-1:1;
    return aa[i]<bb[i]?-1:1;
  }
  return 0;
}
export function presetSnapshot(input,scheme,version){
  exact(input,['schema','profile','values','defaults']);
  const contract=contracts[scheme];
  need(input.schema===1 && contract?.profile===input.profile,'invalid_preset_protocol');
  exact(input.values,Object.keys(contract.fields));exact(input.defaults,Object.keys(contract.fields));
  const keys=Object.keys(input.values).sort();
  need(keys.length>0 && keys.length<=192 && JSON.stringify(keys)===JSON.stringify(Object.keys(input.defaults).sort()),'invalid_preset_snapshot');
  for(const key of keys){
    const f=contract.fields[key];
    need((!f.since || compare(version,f.since)>=0)&&(!f.until || compare(version,f.until)<0),'unsupported_preset_version');
    for(const value of [input.values[key],input.defaults[key]]){
      need(typeof value==='string' && value.length>0 && value.length<=64 && value.trim()===value,'invalid_preset_value');
      const numeric=f.number && /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/.test(value);
      const n=Number(value);
      need(f.values?.includes(value) || f.inherit && value==='inherit' || numeric && Number.isFinite(n)
        && (!f.integer || /^\d+$/.test(value)) && (f.min==null || n>=f.min) && (f.max==null || n<=f.max),'invalid_preset_value');
    }
  }
  if(input.profile==='mfg_vulkan_sm86_7') for(const values of [input.values,input.defaults]){
    need(Number(values.force_multiplier||0)<=Number(values.max_interpolated_frames||5)+1,'invalid_preset_combination');
  }
  return {schema:1,profile:input.profile,values:Object.fromEntries(keys.map(k=>[k,input.values[k]])),defaults:Object.fromEntries(keys.map(k=>[k,input.defaults[k]]))};
}
