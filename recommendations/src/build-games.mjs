// Run from any directory after updating the reviewed client name catalog.
import { readFileSync, writeFileSync } from 'node:fs';
const source=new URL('../../rust/assets/game-names.json',import.meta.url);
const {entries}=JSON.parse(readFileSync(source,'utf8'));
writeFileSync(new URL('./game-directory.json',import.meta.url),JSON.stringify({entries})+'\n','utf8');
console.log(`Prepared ${entries.length} reviewed game identities.`);
