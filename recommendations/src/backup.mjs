import { DatabaseSync, backup } from 'node:sqlite';
import { mkdirSync, chmodSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
const path=resolve(process.env.DATA_FILE || './data/recommendations.sqlite');
const destination=resolve(dirname(path),'backups',new Date().toISOString().replace(/[:.]/g,'-')+'.sqlite');
mkdirSync(dirname(destination),{recursive:true,mode:0o700});
const db=new DatabaseSync(path,{readOnly:true});
try { await backup(db,destination);chmodSync(destination,0o600);console.log('backup_complete'); }
finally {db.close();}
