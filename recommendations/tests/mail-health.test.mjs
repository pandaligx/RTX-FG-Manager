import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mailHealth} from '../src/mail-health.mjs';

test('Cached SMTP probes fail closed, detect authentication failure, recover and expire',async()=>{
  let time=1000,fail=false,calls=0;
  const m=mailHealth({now:()=>time,verify:async()=>{calls++;if(fail)throw Error('secret');},sendCode:async()=>{}});
  assert.equal(m.status().ok,false);
  await Promise.all([m.check(),m.check()]);assert.equal(calls,1);assert.equal(m.status().ok,true);
  for(let i=0;i<100;i++)m.status();assert.equal(calls,1);
  fail=true;await m.check();assert.equal(m.status().state,'smtp_unavailable');assert.ok(!JSON.stringify(m.status()).includes('secret'));
  fail=false;await m.check();assert.equal(m.status().ok,true);time+=601;assert.equal(m.status().state,'check_pending');
});
test('Accepted delivery clears failures, SMTP auth alone cannot clear send rejection, restart retains it',async()=>{
  let stored=null,fail=true,available=true;
  const options={verify:async()=>{},sendCode:async()=>{if(fail)throw Object.assign(Error('private reply'),{code:'EMESSAGE'});},read:()=>stored,write:v=>stored=v,quota:()=>available};
  let m=mailHealth(options);await m.check();
  await assert.rejects(m.sendCode('private@example.com','123456'));await m.check();assert.equal(m.status().state,'delivery_failed');
  m=mailHealth(options);await m.check();assert.equal(m.status().state,'delivery_failed');assert.ok(!JSON.stringify(stored).includes('private'));
  fail=false;await m.sendCode('private@example.com','123456');assert.equal(m.status().ok,true);
  available=false;assert.equal(m.status().state,'daily_limit');available=true;assert.equal(m.status().ok,true);
});
test('Recipient rejection is not a global outage but send-time authentication failure is',async()=>{
  let code='EENVELOPE';
  const m=mailHealth({verify:async()=>{},sendCode:async()=>{throw Object.assign(Error('private'),{code,command:'RCPT TO',responseCode:550});}});
  await m.check();await assert.rejects(m.sendCode('invalid@example.com','123456'));assert.equal(m.status().ok,true);
  code='EAUTH';await assert.rejects(m.sendCode('private@example.com','123456'));assert.equal(m.status().ok,false);
  await m.check();assert.equal(m.status().ok,true);
});
