// Run once on the server. The generated secret never leaves its private env file.
import { randomBytes } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { email } from './contracts.mjs';
const sender=email(process.argv[2] || ''), admin=email(process.argv[3] || sender);
const content=readFileSync(new URL('../config.example',import.meta.url),'utf8')
  .replace(/^AUTH_SECRET=$/m,'AUTH_SECRET='+randomBytes(32).toString('hex'))
  .replace(/^SMTP_USER=$/m,'SMTP_USER='+sender).replace(/^SMTP_FROM=$/m,'SMTP_FROM='+sender)
  .replace(/^ADMIN_EMAIL=$/m,'ADMIN_EMAIL='+admin);
writeFileSync(new URL('../.env',import.meta.url),content,{flag:'wx',mode:0o600});
console.log('Private configuration created. Set SMTP_PASSWORD on this server before starting.');
