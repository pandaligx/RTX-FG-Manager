PRAGMA foreign_keys = ON;
CREATE TABLE identities (
 id TEXT PRIMARY KEY, public_key TEXT NOT NULL UNIQUE,
 verified_at INTEGER NOT NULL, blocked INTEGER NOT NULL DEFAULT 0 CHECK(blocked IN (0,1))
);
CREATE TABLE games (
 id TEXT PRIMARY KEY, name TEXT NOT NULL,
 created_at INTEGER NOT NULL
);
CREATE TABLE profiles (
 id TEXT PRIMARY KEY, author TEXT NOT NULL REFERENCES identities(id),
 game TEXT NOT NULL REFERENCES games(id), scheme TEXT NOT NULL, protocol TEXT NOT NULL,
 revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
 status TEXT NOT NULL DEFAULT 'visible' CHECK(status IN ('visible','review','hidden')),
 official INTEGER NOT NULL DEFAULT 0 CHECK(official IN (0,1)),
 created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE INDEX profile_lookup ON profiles(game,scheme,protocol,status,updated_at);
CREATE INDEX profile_author_game ON profiles(author,game);
CREATE TRIGGER profile_quota BEFORE INSERT ON profiles
WHEN (SELECT COUNT(*) FROM profiles WHERE author=NEW.author AND game=NEW.game)>=5
BEGIN SELECT RAISE(ABORT,'profile_quota'); END;
CREATE TABLE revisions (
 profile TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
 revision INTEGER NOT NULL, content TEXT NOT NULL, created_at INTEGER NOT NULL,
 PRIMARY KEY(profile,revision)
);
CREATE TABLE votes (
 profile TEXT NOT NULL, revision INTEGER NOT NULL,
 identity TEXT NOT NULL REFERENCES identities(id), kind TEXT NOT NULL CHECK(kind IN ('like','works')),
 PRIMARY KEY(profile,revision,identity,kind),
 FOREIGN KEY(profile,revision) REFERENCES revisions(profile,revision) ON DELETE CASCADE
);
CREATE TABLE reports (
 profile TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
 identity TEXT NOT NULL REFERENCES identities(id), revision INTEGER NOT NULL,
 reason TEXT NOT NULL, created_at INTEGER NOT NULL,
 PRIMARY KEY(profile,identity)
);
CREATE TRIGGER report_hide AFTER INSERT ON reports
WHEN (SELECT COUNT(*) FROM reports r JOIN identities i ON i.id=r.identity
 WHERE r.profile=NEW.profile AND i.blocked=0 AND i.verified_at>0)>=3
BEGIN UPDATE profiles SET status='review', official=0 WHERE id=NEW.profile AND status='visible'; END;
-- Removing a report does not automatically reverse moderation or review status.
CREATE TABLE nonces (
 identity TEXT NOT NULL REFERENCES identities(id), nonce TEXT NOT NULL,
 expires_at INTEGER NOT NULL, PRIMARY KEY(identity,nonce)
);
CREATE INDEX nonce_expiry ON nonces(expires_at);
CREATE TRIGGER nonce_identity_allowed BEFORE INSERT ON nonces
WHEN (SELECT blocked FROM identities WHERE id=NEW.identity)!=0
BEGIN SELECT RAISE(ABORT,'identity_blocked'); END;
CREATE TABLE rate_buckets (
 identity TEXT NOT NULL REFERENCES identities(id), minute INTEGER NOT NULL,
 count INTEGER NOT NULL CHECK(count<=30), PRIMARY KEY(identity,minute)
);
CREATE TABLE audit (
 id INTEGER PRIMARY KEY AUTOINCREMENT, actor TEXT NOT NULL, action TEXT NOT NULL,
 target TEXT NOT NULL, reason TEXT NOT NULL, created_at INTEGER NOT NULL
);
