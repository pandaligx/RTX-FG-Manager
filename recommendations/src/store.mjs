import { DatabaseSync } from 'node:sqlite';
import { need } from './contracts.mjs';

export function openStore(path) {
  const db = new DatabaseSync(path);
  const version=db.prepare('PRAGMA user_version').get().user_version;
  need(version <= 5, 'database_newer_than_service', 503);
  db.exec(`PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;
    CREATE TABLE IF NOT EXISTS users(id TEXT PRIMARY KEY,email TEXT UNIQUE NOT NULL,blocked INTEGER NOT NULL DEFAULT 0,created INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS sessions(hash TEXT PRIMARY KEY,user TEXT NOT NULL REFERENCES users(id),expires INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS codes(id TEXT PRIMARY KEY,email TEXT NOT NULL,hash TEXT NOT NULL,expires INTEGER NOT NULL,attempts INTEGER NOT NULL DEFAULT 0);
    CREATE TABLE IF NOT EXISTS limits(key TEXT NOT NULL,bucket INTEGER NOT NULL,count INTEGER NOT NULL,PRIMARY KEY(key,bucket));
    CREATE TABLE IF NOT EXISTS recipes(id TEXT PRIMARY KEY,author TEXT NOT NULL REFERENCES users(id),game TEXT NOT NULL,
      revision INTEGER NOT NULL DEFAULT 1,status TEXT NOT NULL DEFAULT 'visible',content TEXT NOT NULL,updated INTEGER NOT NULL,
      submission TEXT NOT NULL,UNIQUE(author,submission));
    CREATE INDEX IF NOT EXISTS recipes_game ON recipes(game,status,updated);
    CREATE TABLE IF NOT EXISTS likes(recipe TEXT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
      revision INTEGER NOT NULL,user TEXT NOT NULL REFERENCES users(id),PRIMARY KEY(recipe,revision,user));
    CREATE TABLE IF NOT EXISTS dislikes(recipe TEXT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
      revision INTEGER NOT NULL,user TEXT NOT NULL REFERENCES users(id),PRIMARY KEY(recipe,revision,user));
    CREATE TABLE IF NOT EXISTS reports(recipe TEXT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
      user TEXT NOT NULL REFERENCES users(id),reason TEXT NOT NULL,created INTEGER NOT NULL,PRIMARY KEY(recipe,user));
    CREATE TABLE IF NOT EXISTS audit(id INTEGER PRIMARY KEY,actor TEXT NOT NULL,action TEXT NOT NULL,target TEXT NOT NULL,created INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS service_state(key TEXT PRIMARY KEY,value TEXT NOT NULL);`);
  if(version<3) transaction(db,()=>{
    db.exec(`ALTER TABLE reports ADD COLUMN active INTEGER NOT NULL DEFAULT 1;
      ALTER TABLE reports ADD COLUMN reviewed_revision INTEGER NOT NULL DEFAULT 0;
      PRAGMA user_version=3;`);
  });
  if(version<4) transaction(db,()=>{
    db.exec(`CREATE TABLE game_keys(id TEXT PRIMARY KEY,canonical TEXT NOT NULL,name TEXT NOT NULL,original TEXT NOT NULL);
      CREATE INDEX game_keys_canonical ON game_keys(canonical);
      CREATE TABLE game_mapping_changes(id INTEGER PRIMARY KEY,source TEXT NOT NULL,target TEXT NOT NULL,changes TEXT NOT NULL,created INTEGER NOT NULL,actor TEXT NOT NULL,undone INTEGER NOT NULL DEFAULT 0);
      PRAGMA user_version=4;`);
  });
  if(version<5) transaction(db,()=>{
    // Older revisions did not retain the original publication date. Do not invent it.
    db.exec(`ALTER TABLE recipes ADD COLUMN created INTEGER;
      UPDATE recipes SET created=updated WHERE revision=1;
      PRAGMA user_version=5;`);
  });
  return db;
}
export function transaction(db, operation) {
  db.exec('BEGIN IMMEDIATE');
  try { const value = operation(); db.exec('COMMIT'); return value; }
  catch (error) { db.exec('ROLLBACK'); throw error; }
}
export function limit(db, key, now, seconds, max) {
  const bucket = Math.floor(now / seconds) * seconds;
  transaction(db, () => {
    const old = db.prepare('SELECT count FROM limits WHERE key=? AND bucket=?').get(key,bucket);
    need(!old || old.count < max, 'rate_limited', 429);
    db.prepare('INSERT INTO limits VALUES(?,?,1) ON CONFLICT(key,bucket) DO UPDATE SET count=count+1').run(key,bucket);
  });
}
export function cleanExpired(db, now) {
  transaction(db, () => {
    db.prepare('DELETE FROM sessions WHERE expires<?').run(now);
    db.prepare('DELETE FROM codes WHERE expires<?').run(now);
    db.prepare('DELETE FROM limits WHERE bucket<?').run(now-2*86400);
  });
}
export const selectRecipes = `SELECT r.*,(SELECT COUNT(*) FROM likes l JOIN users u ON l.user=u.id
  WHERE l.recipe=r.id AND l.revision=r.revision AND u.blocked=0) likes,
  (SELECT COUNT(*) FROM dislikes l JOIN users u ON l.user=u.id
  WHERE l.recipe=r.id AND l.revision=r.revision AND u.blocked=0) dislikes
  FROM recipes r JOIN users author ON author.id=r.author`;
export function publicRecipe(db,row,user,feedback=false,reporting=false,dates=false) {
  return { id:row.id,author:row.author,revision:row.revision,updatedAt:row.updated,likes:Number(row.likes || 0),
    ...(dates ? {createdAt:row.created ?? null} : {}),
    liked:!!(user && db.prepare('SELECT 1 FROM likes WHERE recipe=? AND revision=? AND user=?').get(row.id,row.revision,user.id)),
    ...(feedback ? {dislikes:Number(row.dislikes || 0),disliked:!!(user && db.prepare('SELECT 1 FROM dislikes WHERE recipe=? AND revision=? AND user=?').get(row.id,row.revision,user.id))} : {}),
    ...(reporting ? {reported:!!(user && db.prepare('SELECT 1 FROM reports WHERE recipe=? AND user=? AND active=1').get(row.id,user.id))} : {}),
    mine:user?.id===row.author,status:row.status,content:JSON.parse(row.content) };
}
export function deleteRecipe(db,id,now) {
  db.prepare("UPDATE recipes SET status='deleted',content='{}',revision=revision+1,updated=? WHERE id=?").run(now,id);
  for(const table of ['likes','dislikes','reports']) db.prepare(`DELETE FROM ${table} WHERE recipe=?`).run(id);
}
