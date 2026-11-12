(() => {
  'use strict';
  const status=document.querySelector('#status'), finish=document.querySelector('#finish'), retry=document.querySelector('#retry');
  const args=new URLSearchParams(location.hash.slice(1)), port=args.get('port'), state=args.get('state');
  history.replaceState(null,'',location.pathname);
  if (!/^\d{4,5}$/.test(port||'') || Number(port)<1024 || Number(port)>65535 || !/^[a-f0-9]{64}$/.test(state||'')) {
    status.textContent='请从管理器的社区页面发起验证。'; return;
  }
  let token='', widget, ended=false;
  const fail=message=>{token='';finish.hidden=true;status.textContent=message;if(!ended)retry.hidden=false;};
  const deadline=setTimeout(()=>{
    ended=true;token='';finish.hidden=true;retry.hidden=true;
    status.textContent='本次验证已超时。请回到管理器，重新点击“首次发布：验证身份”；刷新此页无法重新发起。';
    if(widget!==undefined)window.turnstile?.remove(widget);
  },300000);
  retry.addEventListener('click',()=>{
    if(ended)return;
    token='';finish.hidden=true;retry.hidden=true;status.textContent='正在重新加载验证，请保持管理器打开…';
    if(widget!==undefined)window.turnstile.reset(widget);else prepare();
  });
  finish.addEventListener('click',()=>{
    if(!token||ended) return;
    const form=document.createElement('form');form.method='POST';form.action=`http://127.0.0.1:${port}/verify`;
    for(const [name,value] of Object.entries({state,token})) {const input=document.createElement('input');input.type='hidden';input.name=name;input.value=value;form.append(input);}
    document.body.append(form);form.submit();token='';ended=true;clearTimeout(deadline);finish.disabled=true;retry.hidden=true;
  });
  function prepare(){fetch('/v1/verification-config',{cache:'no-store'}).then(r=>{if(!r.ok)throw Error();return r.json();}).then(config=>{
    if(ended)return;
    if(typeof config.siteKey!=='string'||!config.siteKey)throw Error();
    window.rtxTurnstileReady=()=>{if(!ended)widget=window.turnstile.render('#challenge',{sitekey:config.siteKey,action:'register',retry:'never','refresh-timeout':'manual',
      callback:value=>{if(ended)return;token=value;status.textContent='验证已完成。点击下方按钮返回管理器。';finish.hidden=false;retry.hidden=true;},
      'expired-callback':()=>fail('验证已过期，请点击“重试验证”。'),
      'timeout-callback':()=>fail('验证等待超时，请点击“重试验证”。'),
      'error-callback':code=>{const safe=/^\d{6}$/.test(String(code))?String(code):'未知';fail(`Cloudflare 验证未通过（${safe}）。可重试一次；若仍失败，请检查浏览器是否拦截 challenges.cloudflare.com，或手动换用其他浏览器/网络后从管理器重新发起。`);return true;}});};
    if(window.turnstile)window.rtxTurnstileReady();else{
      const script=document.createElement('script');script.src='https://challenges.cloudflare.com/turnstile/v0/api.js?onload=rtxTurnstileReady&render=explicit';script.async=true;script.onerror=()=>{script.remove();fail('无法连接 Cloudflare 验证服务。请检查网络后点击“重试验证”。');};document.head.append(script);
    }
    status.textContent='请完成下方验证。';
  }).catch(()=>fail('社区验证服务暂不可用，请点击“重试验证”。'));}
  prepare();
})();
