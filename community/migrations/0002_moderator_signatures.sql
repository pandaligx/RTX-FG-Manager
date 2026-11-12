-- Moderation identities are separate from public community registrations.
CREATE TABLE moderator_nonces (
 identity TEXT NOT NULL, nonce TEXT NOT NULL, expires_at INTEGER NOT NULL,
 PRIMARY KEY(identity,nonce)
);
CREATE INDEX moderator_nonce_expiry ON moderator_nonces(expires_at);
