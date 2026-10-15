import { requireThat, authenticate } from './security.mjs';
// Provisioned out of band by the owner; public registration never grants access.
export function moderatorKeys(env) {
  try {const keys=JSON.parse(env.MODERATOR_KEYS || '{}');return keys && typeof keys==='object' && !Array.isArray(keys) ? keys : {};}
  catch {return {};}
}
export async function moderator(request,env,body) {
  const keys=moderatorKeys(env);
  requireThat(Object.keys(keys).length>0,'moderation_not_configured',503);
  const id=request.headers.get('x-rtx-identity') || '';
  requireThat(/^[a-f0-9]{64}$/.test(id) && typeof keys[id]==='string','admin_login_required',401);
  return authenticate(request,body,keys[id]);
}
