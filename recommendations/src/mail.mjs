import nodemailer from 'nodemailer';
import { email, need } from './contracts.mjs';

export function smtpSender(env) {
  const host=env.SMTP_HOST, port=Number(env.SMTP_PORT || 465);
  need(host && [465,587].includes(port) && env.SMTP_USER && env.SMTP_PASSWORD,'smtp_not_configured',503);
  const from=email(env.SMTP_FROM || env.SMTP_USER);
  const transport=nodemailer.createTransport({ host,port,secure:port===465,requireTLS:true,
    auth:{user:env.SMTP_USER,pass:env.SMTP_PASSWORD},pool:true,maxConnections:2,
    connectionTimeout:10000,greetingTimeout:10000,socketTimeout:20000,
    disableFileAccess:true,disableUrlAccess:true });
  return {verify:()=>transport.verify(),close:()=>transport.close(),sendCode:async (to,code) => {
    const result=await transport.sendMail({from:{name:'RTX-FG-Manager',address:from},to,
      subject:'RTX-FG-Manager 登录验证码',
      text:`你的验证码是 ${code}，10 分钟内有效。\n仅用于登录玩家推荐方案，请勿转发。\n如果你没有请求登录，请忽略此邮件。\nYour sign-in code is ${code}. It expires in 10 minutes. Do not share this code.`,
    });
    need(result.accepted?.length>0,'mail_not_accepted',503);
  }};
}
