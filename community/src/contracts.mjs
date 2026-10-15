import { requireThat, safeText } from './security.mjs';
import { protocols } from './protocols.mjs';
const proxies = ['version.dll','dinput8.dll','winmm.dll','dxgi.dll','d3d9.dll','d3d10.dll','d3d11.dll','d3d12.dll','dsound.dll','wininet.dll','winhttp.dll','binkw64.dll','bink2w64.dll','xinput1_1.dll','xinput1_2.dll','xinput1_3.dll','xinput1_4.dll','xinput9_1_0.dll','xinputuap.dll'];
const entries = {
  'dlssg-transfusion-1.4.5.3': [...proxies, 'rtx-encore.asi'],
  'rtx40mfg-1.3.3-hf2': [...proxies, 'OptiScaler/plugins/MFGUnlocker.asi'],
  'upstream-0.3.5-310-9': ['version.dll','winmm.dll','dbghelp.dll','dinput8.dll','dxgi.dll','d3d12.dll'],
};
export function validateSettings(scheme, protocol, settings) {
  const fields = protocols[scheme]?.[protocol];
  requireThat(fields, 'unsupported_protocol');
  requireThat(settings && typeof settings === 'object' && !Array.isArray(settings), 'settings_required');
  const entries = Object.entries(settings);
  requireThat(entries.length > 0 && entries.length <= 128, 'invalid_settings_count');
  const result = {};
  for (const [key, value] of entries) {
    const field = fields[key];
    requireThat(field && typeof value === 'string' && value.length <= 128, 'unsupported_setting:' + key);
    if (field.values) requireThat(field.values.includes(value), 'invalid_setting:' + key);
    else {
      requireThat(/^-?(?:0|[1-9]\d*)(?:\.\d+)?$/.test(value), 'invalid_setting:' + key);
      const number = Number(value);
      requireThat(Number.isFinite(number) && (!field.integer || Number.isInteger(number)) &&
        (field.min === null || number >= field.min) && (field.max === null || number <= field.max), 'invalid_setting:' + key);
    }
    result[key] = value;
  }
  return result;
}
function exactKeys(object, allowed) {
  requireThat(object && typeof object === 'object' && !Array.isArray(object), 'invalid_object');
  requireThat(Object.keys(object).every(key => allowed.includes(key)), 'unsupported_field');
}
function integer(value, min, max) {
  requireThat(Number.isInteger(value) && value >= min && value <= max, 'invalid_number'); return value;
}
export function validateContent(input, scheme, protocol) {
  exactKeys(input, ['title','notes','settings','hardware','patchVersion','entry','runtimeVersions','dlssMode','rayTracing','measurement']);
  const result = { title: safeText(input.title, 80), notes: safeText(input.notes || '', 500), settings: validateSettings(scheme, protocol, input.settings) };
  requireThat(result.title.length > 0, 'title_required');
  if (input.entry != null) {
    requireThat(typeof input.entry === 'string' && entries[scheme]?.includes(input.entry), 'invalid_entry');
    result.entry = input.entry;
  }
  exactKeys(input.hardware || {}, ['gpuModel','driver']);
  result.hardware = {};
  for (const key of ['gpuModel','driver']) if (input.hardware?.[key] != null) result.hardware[key] = safeText(input.hardware[key], 80);
  result.patchVersion = safeText(input.patchVersion, 40);
  requireThat(/^\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?$/.test(result.patchVersion), 'invalid_patch_version');
  exactKeys(input.runtimeVersions || {}, ['fg','sr','rr','streamline']);
  result.runtimeVersions = {};
  for (const [key,value] of Object.entries(input.runtimeVersions || {})) {
    requireThat(typeof value === 'string' && /^\d+(\.\d+){1,3}$/.test(value), 'invalid_runtime_version'); result.runtimeVersions[key] = value;
  }
  for (const key of ['dlssMode','rayTracing']) if (input[key]) result[key] = safeText(input[key], 40);
  if (input.measurement) {
    exactKeys(input.measurement, ['baseFps','outputFps','onePercentLow','scene']);
    const measurement = { source: 'player', scene: safeText(input.measurement.scene || '', 120) };
    for (const key of ['baseFps','outputFps','onePercentLow']) if (input.measurement[key] != null) {
      const n = input.measurement[key]; requireThat(typeof n === 'number' && Number.isFinite(n) && n >= 0 && n <= 10000, 'invalid_measurement'); measurement[key] = n;
    }
    result.measurement = measurement;
  }
  return result;
}
export function validateRoute(game, scheme, protocol) {
  validateGame(game);
  requireThat(protocols[scheme]?.[protocol], 'unsupported_protocol');
}
export function validateGame(game) {
  requireThat(typeof game === 'string' && /^(steam:[1-9]\d{0,9}|game:[a-z0-9][a-z0-9-]{1,79})$/.test(game), 'invalid_game');
}
