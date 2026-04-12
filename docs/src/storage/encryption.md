# At-Rest Encryption (AES-256-GCM)

`github-backup` can encrypt every backup file before uploading to S3 using
**AES-256-GCM** — a widely-audited authenticated encryption scheme from the
[RustCrypto](https://github.com/RustCrypto) suite.

Encryption is **optional** and only applies to S3 uploads.  Local files on disk
are never encrypted by the tool itself; use filesystem-level encryption (LUKS,
BitLocker, etc.) for local security.

---

## Quick Start

```bash
# Generate a random 32-byte key (64 hex chars)
export BACKUP_ENCRYPT_KEY=$(openssl rand -hex 32)

github-backup octocat \
  --token "$GITHUB_TOKEN" \
  --output /var/backup/github \
  --all \
  --s3-bucket my-github-backups \
  --s3-access-key "$AWS_ACCESS_KEY_ID" \
  --s3-secret-key "$AWS_SECRET_ACCESS_KEY"
  # BACKUP_ENCRYPT_KEY is picked up from the environment automatically
```

---

## Key Management

### Generating a Key

```bash
# Generate and print a new random 32-byte key
openssl rand -hex 32
```

Store the key securely (e.g., in AWS Secrets Manager, HashiCorp Vault, or an
encrypted password manager).  **If you lose the key, your encrypted backups
cannot be recovered.**

### Providing the Key

Two ways to supply the encryption key:

| Method | Example |
|--------|---------|
| CLI flag | `--encrypt-key a1b2c3...` (64 hex chars) |
| Environment variable | `BACKUP_ENCRYPT_KEY=a1b2c3...` |

The key is **never written to disk** and **never logged** by the tool.

### Key Rotation

To rotate the key:

1. Download all encrypted objects from S3.
2. Decrypt with the old key (see below).
3. Re-encrypt with the new key and re-upload.

---

## Wire Format

Each encrypted file stored in S3 uses this layout:

```
┌──────────────────────┬────────────────────────────────┐
│  12-byte random nonce│  ciphertext + 16-byte GCM tag  │
└──────────────────────┴────────────────────────────────┘
```

- The **nonce** is generated fresh for every file, ensuring that encrypting the
  same content twice produces different ciphertext.
- The **GCM tag** provides authenticated encryption — any tampering with the
  ciphertext is detected during decryption.
- Encrypted objects receive a `.enc` suffix in S3
  (e.g., `labels.json` → `labels.json.enc`).

---

## Manual Decryption

You can decrypt any `.enc` file without the `github-backup` tool using standard
Unix utilities:

```bash
# Set your key and input file
KEY="your_64_hex_char_key"
INPUT="labels.json.enc"

# Extract nonce (first 12 bytes)
dd if="$INPUT" bs=12 count=1 of=nonce.bin 2>/dev/null

# Extract ciphertext + tag (everything after the first 12 bytes)
dd if="$INPUT" bs=12 skip=1 of=ct.bin 2>/dev/null

# Decrypt with openssl
HEX_NONCE=$(xxd -p nonce.bin | tr -d '\n')
openssl enc -d -aes-256-gcm \
  -K "$KEY" \
  -iv "$HEX_NONCE" \
  -in ct.bin \
  -out labels.json

echo "Decrypted to labels.json"
```

Or using Python with the [`cryptography`](https://pypi.org/project/cryptography/)
package (`pip install cryptography`):

```python
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

key = bytes.fromhex("your_64_hex_char_key")
with open("labels.json.enc", "rb") as f:
    blob = f.read()

nonce, ct_and_tag = blob[:12], blob[12:]
plaintext = AESGCM(key).decrypt(nonce, ct_and_tag, None)

with open("labels.json", "wb") as f:
    f.write(plaintext)
print("Decrypted successfully")
```

---

## Security Notes

- **AES-256-GCM** provides 256-bit key strength and authenticated encryption.
  Any bit flip in the ciphertext or tag causes decryption to fail with an error.
- **Random nonces** — each file gets a fresh 96-bit random nonce via
  `OsRng` (operating-system CSPRNG), providing semantic security.
- **No key derivation** — the 32-byte key is used directly.  If you derive the
  key from a passphrase, use a KDF such as Argon2id or scrypt first.
- **Incremental uploads** — encrypted objects in S3 are identified by their
  `.enc` suffix.  Switching from unencrypted to encrypted (or changing the key)
  will not match existing objects, triggering a fresh upload.
- **S3 server-side encryption** — you can combine client-side AES-256-GCM
  (`--encrypt-key`) with S3 SSE-S3 or SSE-KMS for defence in depth.
