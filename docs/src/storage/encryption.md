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
  (plaintext size + 40 bytes + 16 bytes per started 64 KiB), and upload
  timing. Anyone who can list the bucket sees those.
* **Not protected: swapping and rollback.** The object name is not
  authenticated, so someone with write access to the bucket (but not the key)
  can swap two encrypted objects, or put back an older valid version of one;
  decryption accepts both. They cannot forge new content, nor cut, reorder or
  splice the segments of one object. Use bucket versioning / object lock if
  that matters to you.
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

The key is a 256-bit secret, not a password; if you derive it from a
passphrase, use a KDF such as Argon2id first. It is never used directly to
encrypt new objects: each object is encrypted under its own key derived from it
with HKDF-SHA256 (RFC 5869), and the upload digest below uses another
HKDF-derived subkey, so the key is never used with two primitives.

## Wire Format

Objects written by this version:

```
┌──────────────────┬──────────────────┬───────────┬───────────┬─────┬───────────┐
│ "GHBKENC" + 0x01 │ 32-byte salt     │ segment 0 │ segment 1 │ ... │ segment n │
└──────────────────┴──────────────────┴───────────┴───────────┴─────┴───────────┘
  segment = AES-256-GCM(64 KiB of plaintext) + 16-byte tag; the last may be shorter
```

* This is the **STREAM** construction (Hoang, Reyhanitabar, Rogaway and Vizár,
  CRYPTO 2015) as implemented by RustCrypto's `aead-stream` crate, laid out like
  the payload of the [age](https://age-encryption.org/v1) file format, with
  AES-256-GCM in place of ChaCha20-Poly1305.
* The salt is random (operating-system CSPRNG) for every object. The object's
  key is `HKDF-SHA256(salt, your key, "github-backup-rust/s3/payload/v1")`, so
  every object has its own key and encrypting the same content twice gives
  different ciphertext.
* The 12-byte GCM nonce of segment *i* is seven zero bytes, *i* as a 32-bit
  big-endian number, and one byte that is `1` for the last segment and `0`
  otherwise. Dropping, reordering or cutting off segments makes decryption
  fail. There is no additional authenticated data.
* Because each segment is sealed on its own, a file of any size is encrypted
  and uploaded while holding one part in memory (see
  [Memory use](s3.md#memory-use)).
* Encrypted objects get a `.enc` suffix (`labels.json` becomes
  `labels.json.enc`).

**Objects written by earlier versions** are a single AES-256-GCM message
directly under your key: a 12-byte random nonce, then the ciphertext and a
16-byte tag. They are still decrypted. They carry no header, so `--decrypt`
treats a file that does not start with `GHBKENC` + `0x01` as the old format;
a file that does is decrypted as the new format and, only if that fails
authentication, as the old one. (An old object starts with the magic only by
chance, with probability 2^-64, and is then still read correctly.) Nothing is
ever output that did not authenticate under the key, so the guess cannot yield
wrong plaintext. The new format has a different size, so the first run of this
version **uploads every encrypted object once more** in the new format.

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

`--decrypt` reads the whole object into memory.

Without the tool, any AES-GCM and HKDF implementation can do it. **`openssl
enc` cannot:** it refuses AEAD ciphers ("AEAD ciphers not supported"). With
Python and the [`cryptography`](https://pypi.org/project/cryptography/)
package (both formats):

```python
import sys
from cryptography.exceptions import InvalidTag
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

MAGIC = b"GHBKENC\x01"
SEGMENT = 64 * 1024 + 16  # 64 KiB of plaintext plus its tag


def decrypt(key: bytes, blob: bytes) -> bytes:
    if blob.startswith(MAGIC):
        try:
            k = HKDF(hashes.SHA256(), 32, salt=blob[8:40],
                     info=b"github-backup-rust/s3/payload/v1").derive(key)
            aes, rest, out, i = AESGCM(k), blob[40:], b"", 0
            while True:
                last = len(rest) <= SEGMENT
                nonce = bytes(7) + i.to_bytes(4, "big") + bytes([last])
                out += aes.decrypt(nonce, rest[:SEGMENT], None)
                if last:
                    return out
                rest, i = rest[SEGMENT:], i + 1
        except InvalidTag:
            pass  # not version 1: an object written by an earlier version
    return AESGCM(key).decrypt(blob[:12], blob[12:], None)


key = bytes.fromhex(sys.argv[1])
blob = open(sys.argv[2], "rb").read()
plaintext = decrypt(key, blob)
open(sys.argv[3], "wb").write(plaintext)
print(len(blob), len(plaintext))
```

Run it as `python3 decrypt.py <64-hex-key> labels.json.enc labels.json`. It
was run against two objects produced by the tool's `encrypt()` (key `42`
repeated 32 times): `{"hello":"world"}\n` (a 74-byte object) and a
150 000-byte file spanning three segments, and printed:

```
74 18
150088 150000
```

## Security Notes

* **AES-256-GCM** gives 256-bit key strength and authenticated encryption; any
  change to the ciphertext or tag makes decryption fail.
* **Random salts** come from the operating system (`getrandom`). With a fresh
  256-bit salt per object, two objects end up with the same key (and so reuse
  GCM nonces) with negligible probability, which is why the nonce prefix can
  be fixed.
* You can combine this with server-side encryption (SSE-S3 / SSE-KMS) for
  defence in depth; that protects the disks, not the names.
