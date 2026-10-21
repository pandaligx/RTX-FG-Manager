import { mkdirSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { openStore } from './store.mjs';
import { createApi } from './api.mjs';
import { httpServer } from './http.mjs';
import { smtpSender } from './mail.mjs';
import { mailHealth } from './mail-health.mjs';

const env=process.env, root=dirname(fileURLToPath(import.meta.url));
const path=resolve(env.DATA_FILE || './data/recommendations.sqlite');
mkdirSync(dirname(path),{recursive:true,mode:0o700});
const db=openStore(path);
const dailyMailLimit=Number(env.DAILY_MAIL_LIMIT || 100),smtp=smtpSender(env);
const mail=mailHealth({...smtp,
  read:()=>JSON.parse(db.prepare("SELECT value FROM service_state WHERE key='mail'").get()?.value || 'null'),
  write:state=>db.prepare("INSERT INTO service_state VALUES('mail',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value").run(JSON.stringify(state)),
  quota:()=>Number(db.prepare("SELECT count FROM limits WHERE key='mail-total' AND bucket=?").get(Math.floor(Date.now()/86400000)*86400)?.count || 0)<dailyMailLimit,
});
const api=createApi(db,{secret:env.AUTH_SECRET,adminEmail:env.ADMIN_EMAIL,
  dailyMailLimit,sendCode:mail.sendCode,mailHealth:mail.status,
  games:JSON.parse(readFileSync(resolve(root,'game-directory.json'),'utf8')).entries});
await mail.check();
const probeTimer=setInterval(()=>{void mail.check();},300000);probeTimer.unref();
const assets=new Map([
  ['/admin',{type:'text/html; charset=utf-8',bytes:readFileSync(resolve(root,'../public/admin.html'))}],
  ['/admin.js',{type:'application/javascript; charset=utf-8',bytes:readFileSync(resolve(root,'../public/admin.js'))}],
  ['/admin.css',{type:'text/css; charset=utf-8',bytes:readFileSync(resolve(root,'../public/admin.css'))}],
]);
assets.set('/',assets.get('/admin'));
const server=httpServer(api,{origin:env.PUBLIC_ORIGIN || 'https://recommend.lgxng.cn',trustProxy:env.TRUST_PROXY==='1',assets});
server.listen(Number(env.PORT || 8080),env.HOST || '127.0.0.1',()=>console.log('recommendations_ready'));
for(const signal of ['SIGINT','SIGTERM']) process.on(signal,()=>{
  clearInterval(probeTimer);smtp.close();
  server.close(()=>{db.close();process.exit(0);}); setTimeout(()=>process.exit(1),10000).unref();
});
