# At-Rest Encryption (AES-256-GCM)

`github-backup` can encrypt every file before it is uploaded to S3 with
**AES-256-GCM**, using the [RustCrypto](https://github.com/RustCrypto)
`aes-gcm` crate. Encryption is **optional** and applies **only to S3
uploads**; local files are never encrypted by the tool (use LUKS or similar
for that). The crate is not a FIPS-validated module; the algorithm and mode
are standard (NIST SP 800-38D).

## Quick Start

```bash
export BACKUP_ENCRYPT_KEY=$(openssl rand -hex 32)   # 64 hex characters
export AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=...

github-backup octocat --output /var/backup/github --all \
  --s3-bucket my-github-backups
```

Store the key somewhere other than the machine that holds the backups (a
password manager, a secrets manager). **If you lose the key, the encrypted
objects cannot be recovered.**

## What Is and Is Not Protected

* **Protected:** the *contents* of every uploaded file, including tamper
  detection (any change to an object makes decryption fail).
* **Not protected:** object **names** (they contain your private repository
  names, for example `.../repos/<repo>/issues.json.enc`), object **sizes**
  (plaintext size + 28 bytes), and upload timing. Anyone who can list the
  bucket sees those.
* **Not protected: swapping and rollback.** No associated data is
  authenticated, so someone with write access to the bucket (but not the key)
  can swap two encrypted objects, or put back an older valid version of one;
  decryption accepts both. They cannot forge new content. Use bucket
  versioning / object lock if that matters to you.
* **Local files and git clones** are not encrypted by this feature (and clones
  are not uploaded at all).
* **Enabling encryption later does not remove the earlier plaintext
  objects.** They stay readable in the bucket until you delete them; running
  once with `--s3-delete-stale` removes the plaintext objects that now have
  an encrypted counterpart, or delete them yourself.

## Key Management

| Method | Example |
|--------|---------|
| Environment variable (recommended) | `BACKUP_ENCRYPT_KEY=a1b2c3...` |
| CLI flag (visible in `ps`) | `--encrypt-key a1b2c3...` |

The key is not written to disk by the tool and error messages never contain
any of its characters. What is wiped from memory: the decoded 32-byte key
buffer, the copy shared by the upload tasks, and the AES round keys inside the
cipher (the `aes` crate's `zeroize` feature is enabled). What is **not**
wiped: the hex string held by the argument parser, the process environment and
command line, and any copy the operating system makes (swap, core dumps).

The key is used directly as the AES-256 key (no key derivation); if you derive
it from a passphrase, use a KDF such as Argon2id first. A subkey derived from
it with HKDF-Expand (RFC 5869) is used for the upload digest below, so the AES
key itself is never used with another primitive.

## Wire Format

```
┌──────────────────────┬────────────────────────────────┐
│  12-byte random nonce│  ciphertext + 16-byte GCM tag  │
└──────────────────────┴────────────────────────────────┘
```

* A fresh random 96-bit nonce (operating-system CSPRNG) is generated for every
  object; encrypting the same content twice gives different ciphertext.
* The tag authenticates the ciphertext; there is no additional authenticated
  data.
* Encrypted objects get a `.enc` suffix (`labels.json` becomes
  `labels.json.enc`).
* The whole file is encrypted as a single message, so it is held in memory
  while it is processed. NIST allows one key to encrypt up to 2^32 random-nonce
  messages (collision probability about n^2 / 2^97); a backup tool stays many
  orders of magnitude below that. A single message is limited to about 64 GiB
  by the cipher implementation.

## Skipping Unchanged Files, and Key Rotation

Each encrypted object carries `x-amz-meta-sha256`, which for encrypted uploads
is **HMAC-SHA256 of the plaintext under a key derived from your encryption
key**, never a plain hash. Consequences:

* An unchanged file is skipped; a changed file (even with the same size) is
  uploaded again.
* Someone with read access to the bucket learns *that two plaintexts are
  equal* (equal digests) but cannot test a guess about their content.
* **Changing the key changes every digest**, so the next run uploads every
  file again, encrypted under the new key, overwriting the old objects.

### Rotating the key

There is no automatic re-encryption of what is already in the bucket, but the
steps are short:

1. Generate a new key and keep the old one until the end.
2. Run the backup with the **new** key (`BACKUP_ENCRYPT_KEY=<new>`). Every
   file whose local copy still exists is uploaded again under the new key.
3. Verify: decrypt one or two objects with the new key (see below).
4. Objects whose local file no longer exists (and, with `--s3-delete-stale`
   not set, anything removed locally) remain encrypted under the **old** key.
   Either run once with `--s3-delete-stale` to remove them, or keep the old key
   for as long as you need them.
5. Retire the old key only when no object remains that needs it, and only
   after step 3 succeeded. Bucket versions (if versioning is on) of the old
   objects stay encrypted under the old key.

## Decrypting

Use the tool itself (works on the objects this version and earlier versions
produced):

```bash
export BACKUP_ENCRYPT_KEY=...   # the key used for the upload
github-backup --decrypt \
  --decrypt-input issues.json.enc --decrypt-output issues.json
```

> `--decrypt` needs neither an `OWNER` nor network access, only the key and the
> two file paths.  Download objects with your provider's tool first (the tool
> itself has no download command), then decrypt one file at a time; for many
> files loop in the shell.

Without the tool, any AES-GCM implementation can do it. **`openssl enc` cannot:**
it refuses AEAD ciphers ("AEAD ciphers not supported"). With Python and the
[`cryptography`](https://pypi.org/project/cryptography/) package:

```python
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

key = bytes.fromhex("your_64_hex_char_key")
with open("labels.json.enc", "rb") as f:
    blob = f.read()
plaintext = AESGCM(key).decrypt(blob[:12], blob[12:], None)
open("labels.json", "wb").write(plaintext)
```

This snippet was run against a file produced by the tool's `encrypt()`
(key `42` repeated 32 times, plaintext `{"hello":"world"}\n`, 46-byte object):

```
46 b'{"hello":"world"}\n'
```

## Security Notes

* **AES-256-GCM** gives 256-bit key strength and authenticated encryption; any
  change to the ciphertext or tag makes decryption fail.
* **Random nonces** come from `OsRng`.
* You can combine this with server-side encryption (SSE-S3 / SSE-KMS) for
  defence in depth; that protects the disks, not the names.
