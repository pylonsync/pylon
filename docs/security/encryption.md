# Pylon: `field.encrypted()` for at-rest AEAD encryption

Mark a manifest field `encrypted()` and Pylon AEAD-encrypts it
before the storage backend sees the value. Reads through any
`ctx.db.*` / `/api/entities/*` / `/api/sync/*` path return
plaintext to authorized callers; a SQL dump or stolen DB file
exposes only ciphertext.

## Quick start

```bash
# Generate a 32-byte key
openssl rand -hex 32 > .pylon-encryption-key

# Set the env on every Pylon process
export PYLON_ENCRYPTION_KEY=$(cat .pylon-encryption-key)
```

```ts
// schema.ts
import { field, entity } from "@pylonsync/sdk";

export default entity("Customer", {
  name: field.string(),
  email: field.string().unique(),
  // Encrypted at rest. Plaintext only exists inside the Pylon process.
  ssn: field.string().serverOnly().encrypted(),
  // Encrypt OAuth tokens and keep them off the wire.
  refreshToken: field.string().serverOnly().encrypted(),
});
```

After deploy, run any mutation that touches the encrypted field once
to migrate legacy plaintext rows; reads of rows written before
`encrypted()` was set pass through transparently (no `enc:v1:`
prefix → no decrypt attempt).

## Threat model

**Protects against**:
- DB file copy, SQL dump leak, backup left in a misconfigured bucket.
- A read-only SQL replica that ends up with broader read access than
  intended.
- Postgres + SQLite both store the ciphertext directly in the column
  cell; no separate keystore is needed.

**Does not protect against**:
- An attacker with code execution on the Pylon process; the key
  lives in env / process memory.
- Side-channel attacks against the AEAD primitive itself. Pylon uses
  `ring`'s constant-time implementations to minimize this.
- Memory-dump attacks (key in RAM during process lifetime).
- Trusted server-side function code that legitimately reads the
  decrypted value. Server-side code is trusted by design.

If your threat model requires KMS-backed or HSM-backed key
material, sub in your own key source by setting
`PYLON_ENCRYPTION_KEY` from a vault-fetch at boot (e.g. AWS Secrets
Manager → systemd `EnvironmentFile`).

## Configuration

The key is loaded once at boot from `PYLON_ENCRYPTION_KEY`. Format
options:

| Format    | Example                                                            |
| --------- | ------------------------------------------------------------------ |
| 64-char hex | `0123456789abcdef...` (use `openssl rand -hex 32`)               |
| Base64     | `AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=` (use `openssl rand -base64 32`) |

The key must decode to exactly 32 bytes. Anything else fails fast at
boot with `ENCRYPTION_NOT_CONFIGURED` instead of silently writing
plaintext.

When the manifest declares encrypted fields but `PYLON_ENCRYPTION_KEY`
is unset, writes to those fields reject with `ENCRYPTION_NOT_CONFIGURED`.
This is the safe default: callers see the failure immediately rather
than discovering it on disaster recovery.

## Wire format

Encrypted values on disk look like:

```
enc:v1:<base64(nonce)>:<base64(ciphertext + tag)>
```

- **AEAD**: ChaCha20-Poly1305 (via `ring`).
- **Nonce**: 12 bytes, random per cell (`ring::rand::SystemRandom`).
- **Tag**: Poly1305 16-byte MAC appended to the ciphertext.
- **Version prefix** (`v1`): future rotations (key derivation
  scheme, AEAD swap) will introduce `v2:` and coexist with v1
  records.

The wire format is the value stored in the SQL column. CRDT projection,
FTS shadow tables, change-log events, and sync push/pull all
see the ciphertext as an opaque string, not the plaintext.

## Restrictions enforced at boot

The runtime refuses to start when the manifest declares an encrypted
field with any of these combinations. The intent is to catch
misconfigurations before a single byte hits the disk.

- **Field type must be `string` or `richtext`.** Numeric / boolean /
  datetime columns can't hold the base64 wire format.
- **Cannot combine with `unique: true`.** Random-nonce encryption
  makes uniqueness checks meaningless (same plaintext writes
  different ciphertext every time). Reject at boot.
- **Must combine with `serverOnly: true`.** Without `serverOnly`, the
  decrypted plaintext would ship over every public HTTP/WS surface,
  defeating the whole point. Reject at boot.

## Other limits

- **Queries by value are unsupported.** Because every write uses a fresh nonce,
  two encrypted writes of the same plaintext produce different
  ciphertext. `ctx.db.lookup("Customer", "ssn", "111-22-3333")`
  always returns `null`; the runtime cannot find a row by an
  encrypted column's value. If you need lookup, store a hashed
  version in a separate non-encrypted field.
- **Indexes**: Indexing an encrypted field is allowed but pointless
  (every row's value differs).
- **No partial decryption**: every read of a row carrying an
  encrypted field decrypts every encrypted column on that row.
  For very wide rows with many encrypted columns, this matters in
  the hot path; profile before assuming it doesn't.

## AAD binding

Every cell binds `("pylon-aead-v1:", entity, "\0", field_name)` as
additional authenticated data on the AEAD seal. An attacker who copies
a ciphertext blob from one (row, column) to another in the DB file
cannot make decryption succeed because the tag check fails when the AAD
mismatches.

## Known gaps (tracked, not yet covered)

These paths are not yet wrapped. Writes through them store plaintext, and reads
through them return ciphertext:

- `Runtime::transact` (atomic batch `/api/transact`): the SQLite path
  uses raw `insert_with_conn`/`update_with_conn`, but
  `pg_transact_with_crdt` (Postgres) bypasses the encryption layer
  entirely. Until this is fixed, **do not use `/api/transact` to
  write rows with encrypted fields on a Postgres-backed deploy**.
- `Runtime::query_filtered`, `Runtime::query_graph`, `Runtime::search`
  return rows without decrypting. Callers see
  ciphertext for encrypted fields. Fix: thread `maybe_decrypt_row`
  through these methods.

The wrapper covers standard reads (`ctx.db.get`, `ctx.db.list`,
`ctx.db.lookup`), standard writes (`ctx.db.insert`, `ctx.db.update`), and the
action-handler transaction path (`TxStore`, `PgBufferedTxStore` via
`*_with_conn`). This covers the main access paths on both SQLite and Postgres
deploys.

## Operational notes

### Migrating an existing field to encrypted

Add `.encrypted()` to the schema, deploy, then re-write every row
that has a value in the field. The simplest re-write path:

```ts
// migrations/encrypt_ssn.ts
export default action({
  async handler(ctx) {
    const rows = await ctx.db.list("Customer");
    for (const row of rows) {
      if (typeof row.ssn === "string" && !row.ssn.startsWith("enc:v1:")) {
        await ctx.db.update("Customer", row.id as string, { ssn: row.ssn });
      }
    }
  },
});
```

Reading any row that still carries plaintext returns the plaintext as-is
(the framework's decrypt logic skips values without the `enc:v1:` prefix).
Adding `encrypted()` is a non-breaking deployment. Old rows keep
working until they're rewritten.

### Key rotation

Out of scope for v1. The framework supports a single key; rotating
means re-encrypting every row with the new key (read with the old
key, write with the new). A future release will support key
versioning (`PYLON_ENCRYPTION_KEY_V1`, `PYLON_ENCRYPTION_KEY_V2`) so
old rows can be read while new writes use the new key.

Do not rotate the key until versioned keys are supported. If rotation is
unavoidable, use an offline migration.

### Backup + restore

The ciphertext is fully self-contained (nonce + ciphertext + tag in
one cell). A backup that copies the SQL data + nothing else is
useless without the key. Treat `PYLON_ENCRYPTION_KEY` like a
production secret: store it in your secret manager, outside the repo and away
from your database backups.

## Errors

| Code                        | Meaning                                                                  |
| --------------------------- | ------------------------------------------------------------------------ |
| `ENCRYPTION_NOT_CONFIGURED` | Manifest declares encrypted fields but `PYLON_ENCRYPTION_KEY` is unset. |
| `ENCRYPTION_FAILED`         | AEAD operation rejected (corrupt ciphertext on read, RNG failure on write). |
| `Invalid PYLON_ENCRYPTION_KEY` | Boot-time: the env value isn't 32 bytes hex or base64. Server logs warn; encrypted writes fail until fixed. |
