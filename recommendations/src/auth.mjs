import { createHmac, randomBytes, randomInt, randomUUID, timingSafeEqual } from 'node:crypto';
import { ApiError, need, email, exact, digest, uuid } from './contracts.mjs';
import { limit, transaction } from './store.mjs';

export function authService(db, { secret, sendCode, now, dailyMailLimit = 100, adminEmail = '' }) {
  need(typeof secret === 'string' && secret.length >= 32, 'server_secret_required', 503);
  need(Number.isSafeInteger(dailyMailLimit) && dailyMailLimit>0 && dailyMailLimit<=10000,'invalid_mail_limit',503);
  const hash = value => createHmac('sha256', secret).update(value).digest('hex');
  const fingerprint = value => hash(value).slice(0, 40);
  const admin = adminEmail ? email(adminEmail) : '';
  return {
    fingerprint,
    user(header, required = false) {
      if (!header) { need(!required, 'login_required', 401); return null; }
      need(typeof header === 'string' && /^Bearer [A-Za-z0-9_-]{43}$/.test(header), 'session_expired', 401);
      const user = db.prepare(`SELECT u.*,s.expires FROM sessions s JOIN users u ON u.id=s.user
        WHERE s.hash=? AND s.expires>?`).get(digest(header.slice(7)), now());
      need(user && !user.blocked, 'session_expired', 401);
      return { id: user.id, email: user.email, admin: !!admin && user.email === admin, expires: user.expires };
    },
    async request(body, ip) {
      exact(body, ['email']);
      const address = email(body.email), t = now();
      // Reserve quotas before awaiting mail: concurrent requests cannot overrun them.
      limit(db, 'mail-ip:' + fingerprint(ip), t, 3600, 10);
      limit(db, 'mail-email:' + fingerprint(address), t, 3600, 5);
      limit(db, 'mail-total', t, 86400, dailyMailLimit);
      const id = randomUUID(), code = String(randomInt(0, 1000000)).padStart(6, '0');
      transaction(db, () => {
        const recent = db.prepare('SELECT expires FROM codes WHERE email=? ORDER BY expires DESC LIMIT 1').get(address);
        need(!recent || recent.expires <= t + 540, 'wait_before_resending', 429);
        db.prepare('DELETE FROM codes WHERE email=?').run(address);
        db.prepare('INSERT INTO codes(id,email,hash,expires) VALUES(?,?,?,?)').run(id,address,hash(id + ':' + code),t+600);
      });
      try { await sendCode(address, code); }
      catch {
        db.prepare('DELETE FROM codes WHERE id=?').run(id);
        throw new ApiError('mail_unavailable', 503);
      }
      return { challenge: id, expiresIn: 600, retryAfter: 60 };
    },
    verify(body, ip) {
      exact(body, ['challenge','code']);
      need(uuid(body.challenge) && typeof body.code === 'string' && /^\d{6}$/.test(body.code), 'invalid_code');
      const t = now();
      limit(db, 'verify:' + fingerprint(ip), t, 600, 30);
      // Failed attempts must commit too; throwing inside this transaction would undo them.
      const result = transaction(db, () => {
        const row = db.prepare('SELECT * FROM codes WHERE id=?').get(body.challenge);
        if (!row || row.expires <= t || row.attempts >= 5) return null;
        db.prepare('UPDATE codes SET attempts=attempts+1 WHERE id=?').run(row.id);
        if (!timingSafeEqual(Buffer.from(row.hash,'hex'),Buffer.from(hash(row.id+':'+body.code),'hex'))) return null;
        db.prepare('DELETE FROM codes WHERE email=?').run(row.email);
        let user = db.prepare('SELECT * FROM users WHERE email=?').get(row.email);
        if (user?.blocked) return null;
        if (!user) {
          user = { id: randomUUID(), email: row.email };
          db.prepare('INSERT INTO users(id,email,created) VALUES(?,?,?)').run(user.id,user.email,t);
        }
        const token = randomBytes(32).toString('base64url'), expiresAt = t + 30*86400;
        // Bounded retained sessions, oldest sessions become invalid first.
        db.prepare(`DELETE FROM sessions WHERE user=? AND hash NOT IN
          (SELECT hash FROM sessions WHERE user=? ORDER BY expires DESC LIMIT 4)`).run(user.id,user.id);
        db.prepare('INSERT INTO sessions VALUES(?,?,?)').run(digest(token),user.id,expiresAt);
        return { token, expiresAt, user: { id:user.id, admin:!!admin && user.email===admin } };
      });
      need(result, 'invalid_or_expired_code', 401);
      return result;
    },
    logout(header) { if (header) db.prepare('DELETE FROM sessions WHERE hash=?').run(digest(header.slice(7))); },
  };
}
