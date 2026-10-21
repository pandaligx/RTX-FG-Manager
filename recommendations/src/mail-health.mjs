// SMTP verification checks connection/TLS/authentication, not inbox delivery.
// Only generic state is retained; never store addresses, SMTP replies or OTPs here.
export function mailHealth({verify,sendCode,read=()=>null,write=()=>{},now=()=>Math.floor(Date.now()/1000),quota=()=>true}) {
  let delivery=read() || {failed:false,lastAcceptedAt:null}, probe={ok:false,checkedAt:null}, pending;
  const persist=()=>write(delivery);
  const check=()=>{
    if(pending)return pending;
    pending=Promise.resolve().then(verify)
      .then(()=>{probe={ok:true,checkedAt:now()};},()=>{probe={ok:false,checkedAt:now()};})
      .finally(()=>{pending=null;});
    return pending;
  };
  return {
    check,
    async sendCode(to,code) {
      try {
        await sendCode(to,code);
        delivery={failed:false,lastAcceptedAt:now()};persist();
        probe={ok:true,checkedAt:now()};
      } catch(error) {
        // A bad recipient must not take the whole service monitor down.
        const recipientRejected=error?.code==='EENVELOPE' && /^RCPT\b/i.test(error.command || '') && error.responseCode>=500;
        const connection=['EAUTH','ECONNECTION','ETIMEDOUT','ESOCKET','EDNS','ETLS'].includes(error?.code);
        if(connection)probe={ok:false,checkedAt:now()};
        else if(!recipientRejected){delivery={...delivery,failed:true};persist();}
        throw error;
      }
    },
    status() {
      const fresh=probe.checkedAt!==null && now()-probe.checkedAt<=600;
      const state=!fresh?'check_pending':!probe.ok?'smtp_unavailable':delivery.failed?'delivery_failed':!quota()?'daily_limit':'ready';
      return {ok:state==='ready',state,checkedAt:probe.checkedAt,lastAcceptedAt:delivery.lastAcceptedAt};
    },
  };
}
