import {test} from 'node:test';
import assert from 'node:assert/strict';
import {recipe} from '../src/contracts.mjs';
import {presetMetadata} from '../src/presets.mjs';

test('custom Delta 041 is separate from legacy and accepts only public parameters',()=>{
  const c={schema:1,game:{id:'steam:2507950',name:'Delta Force'},title:'4X',notes:'',
    scheme:'rtxfg-0.4.1-dx12-vulkan',patchVersion:'0.4.1',entries:['version.dll'],renderer:'dx12',
    gpuModel:'RTX 3060',driver:'',gpuAlias:'unchanged',neuralRuntime:'none',
    presets:{schema:1,profile:'upstream041',values:{fg_engine:'XeSS',max_generated_frames:'3',optimized:'1'},
      defaults:{fg_engine:'DLSS',max_generated_frames:'3',optimized:'1'}}};
  assert.deepEqual(recipe(c).presets,c.presets);
  assert.throws(()=>recipe({...c,scheme:'rtxfg-0.3.5-dx12-vulkan'}));
  const bad=structuredClone(c);
  bad.presets.values.DeltaForceRuntimeId='a'.repeat(32);
  bad.presets.defaults.DeltaForceRuntimeId='a'.repeat(32);
  assert.throws(()=>recipe(bad));
  const excessive=structuredClone(c);excessive.presets.values.max_generated_frames='5';
  assert.throws(()=>recipe(excessive));
  assert.equal(presetMetadata()['rtxfg-0.3.5-dx12-vulkan'].profile,'upstream035');
  assert.equal(presetMetadata()['upstream-0.3.5-310-9'].from_041.profile,'upstream041');
});
