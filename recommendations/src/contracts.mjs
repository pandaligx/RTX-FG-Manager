import { createHash } from 'node:crypto';

export class ApiError extends Error {
  constructor(code, status = 400) { super(code); this.status = status; }
}
export function need(value, code, status = 400) { if (!value) throw new ApiError(code, status); }
export function exact(value, keys) {
  need(value && typeof value === 'object' && !Array.isArray(value), 'invalid_object');
  need(Object.keys(value).every(k => keys.includes(k)), 'unsupported_field');
}
export function text(value, limit, empty = false) {
  need(typeof value === 'string' && [...value].length <= limit, 'invalid_text');
  const result = value.trim();
  need(empty || result.length > 0, 'empty_text');
  // Public notes are plain text. Files, links and personal paths are never recipes.
  need(!/[\x00-\x08\x0b-\x1f\x7f<>\u202a-\u202e\u2066-\u2069]/u.test(result)
    && !/(?:[a-z]:[\\/]|\\\\|https?:\/\/|\/Users\/|\/home\/)/i.test(result), 'private_or_unsafe_text');
  return result;
}
export function gameId(value) {
  need(typeof value === 'string' && value.length <= 190, 'invalid_game');
  need(/^(?:steam|gog):[1-9]\d{0,11}$/.test(value)
    || /^(?:epic|ea|ubisoft|xbox|battlenet|wegame|hoyo|cube|sonkwo|gog_slug|epic_slug|catalog):[a-z0-9][a-z0-9_.\/-]{0,159}$/.test(value)
    || /^custom:[a-f0-9]{64}$/.test(value), 'invalid_game');
  need(!value.includes('..'), 'invalid_game');
  return value;
}
export const digest = value => createHash('sha256').update(value).digest('hex');
export function email(value) {
  need(typeof value === 'string' && value.length <= 254, 'invalid_email');
  const result = value.trim().toLowerCase();
  need(/^[a-z0-9.!#$%&'*+\/?^_`{|}~-]{1,64}@[a-z0-9](?:[a-z0-9.-]{0,188}[a-z0-9])?\.[a-z]{2,63}$/.test(result)
    && !result.includes('..'), 'invalid_email');
  return result;
}
export const uuid = value => typeof value === 'string' && /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/.test(value);

// Reviewed loader names only. Download addresses and local paths are absent.
const universal = ['version.dll','dinput8.dll','winmm.dll','dxgi.dll','d3d9.dll','d3d10.dll','d3d11.dll','d3d12.dll','dsound.dll','wininet.dll','winhttp.dll','binkw64.dll','bink2w64.dll','xinput1_1.dll','xinput1_2.dll','xinput1_3.dll','xinput1_4.dll','xinput9_1_0.dll','xinputuap.dll'];
export const schemes = {
  'dlssg-transfusion-1.4.5.3': [...universal, 'rtx-encore.asi'],
  'rtx40mfg-1.3.3-hf2': [...universal, 'OptiScaler/plugins/MFGUnlocker.asi'],
  'upstream-0.3.5-310-9': ['version.dll','winmm.dll','dbghelp.dll','dinput8.dll','dxgi.dll','d3d12.dll'],
  'rtxfg-0.3.5-dx12-vulkan': ['version.dll','winmm.dll','dbghelp.dll','dinput8.dll','dxgi.dll','d3d12.dll'],
  'pipotoufik-mfg-vulkan-sm86-7': ['version.dll'],
};
export function recipe(input) {
  exact(input, ['schema','game','title','notes','scheme','patchVersion','entries','renderer','gpuModel','driver','gpuAlias','neuralRuntime','platform','edition']);
  need(input.schema === 1, 'unsupported_schema');
  exact(input.game, ['id','name']);
  const game = { id: gameId(input.game.id), name: text(input.game.name, 120) };
  if (game.id.startsWith('custom:')) {
    need(game.id === 'custom:' + digest(game.name.replace(/\s+/gu,' ').toLowerCase()), 'game_name_mismatch');
  }
  need(Object.hasOwn(schemes, input.scheme), 'unsupported_scheme');
  need(Array.isArray(input.entries) && input.entries.length > 0 && input.entries.length <= 6
    && new Set(input.entries).size === input.entries.length
    && input.entries.every(e => schemes[input.scheme].includes(e)), 'invalid_entries');
  need(!input.entries.some(e => e.endsWith('.asi')) || input.entries.length === 1, 'plugin_proxy_conflict');
  need(typeof input.patchVersion === 'string' && /^\d+\.\d+\.\d+(?:[-+][a-zA-Z0-9.-]+)?$/.test(input.patchVersion)
    && input.patchVersion.length <= 40, 'invalid_patch_version');
  need(['dx12','vulkan','dx11','unknown'].includes(input.renderer), 'invalid_renderer');
  need(['unchanged','rtx4060','rtx5090','unknown'].includes(input.gpuAlias), 'invalid_gpu_alias');
  need(['none','310.9.16','unknown'].includes(input.neuralRuntime), 'invalid_neural_runtime');
  need(input.platform===undefined || ['unknown','steam','epic','ea','gog','ubisoft','xbox','battlenet','wegame','hoyo','cube','sonkwo','standalone'].includes(input.platform),'invalid_platform');
  return { schema: 1, game, title: text(input.title, 80), notes: text(input.notes || '', 500, true),
    ...(input.platform?{platform:input.platform}:{}),...(input.edition?{edition:text(input.edition,80)}:{}),
    scheme: input.scheme, patchVersion: input.patchVersion, entries: input.entries,
    renderer: input.renderer, gpuModel: text(input.gpuModel || '', 80, true),
    driver: text(input.driver || '', 40, true), gpuAlias: input.gpuAlias, neuralRuntime: input.neuralRuntime };
}
