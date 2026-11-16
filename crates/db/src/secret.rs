//! Secret values in the database, sealed with the server's database key.
//!
//! A value is sealed with AES-256-GCM under a 32-byte key that sherpad keeps in a file outside
//! the database, so the database file and its backups hold only ciphertext. A sealed value is
//! one `BLOB`: a format version byte, the 12-byte nonce, then the ciphertext and its 16-byte tag.
//! Every value gets a fresh random nonce. The value's place, its table, column and row key, is
//! the associated data, so a sealed value copied to another row or column does not open.
//!
//! A `BLOB` column that holds a secret stores `seal`'s output, and its reader calls `open`. A
//! `TEXT` column, such as `user.device_password`, stores a [`SealedText`]: `sealed:` and the
//! sealed bytes in lower-case hex. `SealedText` is made only by [`seal_text`], so a writer that
//! binds one cannot store plain text, and the table's triggers refuse any other value. The
//! `secret_key_check` row proves which key the database's values are sealed with: sherpad
//! checks the key against it at startup ([`verify_secret_key`]).

use std::fmt;

use aes_gcm::aead::{Aead, Nonce, Payload};
use aes_gcm::{Aes256Gcm, KeyInit};
use anyhow::{Context, Result, anyhow, bail};
use rand::RngCore;
use tracing::instrument;
use turso::{Row, Value};

use crate::Database;
use crate::queries;
use crate::sql::{execute, query_optional};

/// The format of a sealed value, its first byte.
const SEALED_VERSION: u8 = 1;
/// The length of an AES-GCM nonce, in bytes.
const NONCE_LENGTH: usize = 12;
/// The length of an AES-GCM tag, in bytes.
const TAG_LENGTH: usize = 16;
/// The one row of `secret_key_check`.
const CHECK_ROW: i64 = 1;
/// The value sealed in `secret_key_check`.
const CHECK_PLAINTEXT: &[u8] = b"sherpa database key";
/// Where the check value is, its associated data.
const CHECK_CONTEXT: &str = "secret_key_check.sealed:1";

/// The prefix of a [`SealedText`].
pub(crate) const SEALED_TEXT_PREFIX: &str = "sealed:";

/// A secret sealed for a `TEXT` column: [`SEALED_TEXT_PREFIX`], then the sealed value in
/// lower-case hex. Only [`seal_text`] makes one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SealedText(String);

impl From<SealedText> for Value {
    fn from(sealed: SealedText) -> Self {
        Value::Text(sealed.0)
    }
}

/// The key that seals the database's secret values: 32 random bytes. It is never stored in the
/// database, and its `Debug` shows no bytes.
#[derive(Clone)]
pub struct SecretKey([u8; SecretKey::LENGTH]);

impl SecretKey {
    /// The length of a key, in bytes.
    pub const LENGTH: usize = 32;

    /// A new random key.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0u8; Self::LENGTH];
        rand::thread_rng().fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// The key in `bytes`, as read from the key file.
    ///
    /// # Errors
    ///
    /// Fails when `bytes` is not 32 bytes long.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let key: [u8; Self::LENGTH] = bytes.try_into().map_err(|_| {
            anyhow!(
                "A database key is {} bytes long, not {}",
                Self::LENGTH,
                bytes.len()
            )
        })?;
        Ok(Self(key))
    }

    /// The key's bytes, to write to the key file.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new(&self.0.into())
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKey(..)")
    }
}

/// Seal `plaintext` for the place named by `context`, `table.column:row`, under `key`.
pub(crate) fn seal(key: &SecretKey, plaintext: &[u8], context: &str) -> Result<Vec<u8>> {
    let mut nonce = [0u8; NONCE_LENGTH];
    rand::thread_rng().fill_bytes(&mut nonce);
    let ciphertext = key
        .cipher()
        .encrypt(
            &Nonce::<Aes256Gcm>::from(nonce),
            Payload {
                msg: plaintext,
                aad: context.as_bytes(),
            },
        )
        .map_err(|_| anyhow!("Failed to seal {context}"))?;
    let mut sealed = Vec::with_capacity(1 + NONCE_LENGTH + ciphertext.len());
    sealed.push(SEALED_VERSION);
    sealed.extend_from_slice(&nonce);
    sealed.extend_from_slice(&ciphertext);
    Ok(sealed)
}

/// Open `sealed`, a value `seal` made for the place named by `context`, under `key`.
///
/// # Errors
///
/// Fails when the value is not in the sealed format, and when it does not authenticate: it was
/// sealed with another key, for another place, or has been changed.
pub(crate) fn open(key: &SecretKey, sealed: &[u8], context: &str) -> Result<Vec<u8>> {
    let Some((&version, rest)) = sealed.split_first() else {
        bail!("{context} is empty, not a sealed value");
    };
    if version != SEALED_VERSION {
        bail!("{context} is sealed in format {version}, which this Sherpa build does not know");
    }
    if rest.len() < NONCE_LENGTH + TAG_LENGTH {
        bail!("{context} is too short to be a sealed value");
    }
    let (nonce, ciphertext) = rest.split_at(NONCE_LENGTH);
    let nonce: [u8; NONCE_LENGTH] = nonce
        .try_into()
        .with_context(|| format!("{context} has no nonce"))?;
    key.cipher()
        .decrypt(
            &Nonce::<Aes256Gcm>::from(nonce),
            Payload {
                msg: ciphertext,
                aad: context.as_bytes(),
            },
        )
        .map_err(|_| {
            anyhow!(
                "Failed to open {context}: it was sealed with a different database key, or has \
                 been changed"
            )
        })
}

/// Seal `plaintext` for the `TEXT` column place named by `context`, under `key`.
pub(crate) fn seal_text(key: &SecretKey, plaintext: &str, context: &str) -> Result<SealedText> {
    let sealed = seal(key, plaintext.as_bytes(), context)?;
    let mut text = String::with_capacity(SEALED_TEXT_PREFIX.len() + sealed.len() * 2);
    text.push_str(SEALED_TEXT_PREFIX);
    for byte in sealed {
        text.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]));
    }
    Ok(SealedText(text))
}

/// Open `stored`, a [`SealedText`] read from the `TEXT` column place named by `context`.
///
/// # Errors
///
/// Fails when `stored` is not a sealed text, as plain text is not, and as `open` does.
pub(crate) fn open_text(key: &SecretKey, stored: &str, context: &str) -> Result<String> {
    let Some(hex) = stored.strip_prefix(SEALED_TEXT_PREFIX) else {
        bail!("{context} is not sealed");
    };
    if hex.len() % 2 != 0 {
        bail!("{context} is not a sealed value: its hex has an odd length");
    }
    let mut sealed = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().chunks(2) {
        let [high, low] = pair else {
            bail!("{context} is not a sealed value");
        };
        sealed.push((hex_value(*high, context)? << 4) | hex_value(*low, context)?);
    }
    String::from_utf8(open(key, &sealed, context)?)
        .with_context(|| format!("{context} is not text"))
}

/// The lower-case hex digits.
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// The value of the lower-case hex digit `digit`.
fn hex_value(digit: u8, context: &str) -> Result<u8> {
    match digit {
        b'0'..=b'9' => Ok(digit - b'0'),
        b'a'..=b'f' => Ok(digit - b'a' + 10),
        _ => bail!("{context} is not a sealed value: it is not lower-case hex"),
    }
}

fn sealed_from_row(row: &Row) -> Result<Vec<u8>> {
    match row.get_value(0)? {
        Value::Blob(bytes) => Ok(bytes),
        other => bail!("secret_key_check.sealed is not a blob: {other:?}"),
    }
}

/// Whether the database's values are sealed with a key already: the `secret_key_check` row
/// exists. sherpad refuses to make a new key for such a database.
///
/// # Errors
///
/// Fails when the database cannot be read.
#[instrument(skip(db), level = "debug")]
pub async fn secret_key_registered(db: &Database) -> Result<bool> {
    let conn = db.reader().await?;
    let sealed = query_optional(
        &conn,
        queries::definition::FIND_SECRET_KEY_CHECK.bind([CHECK_ROW.into()]),
        sealed_from_row,
    )
    .await
    .context("Failed to read the database key check")?;
    Ok(sealed.is_some())
}

/// Check the database's key against the `secret_key_check` row, or write the row with it when
/// the database has none.
///
/// # Errors
///
/// Fails when the database has no key loaded, when the key is not the one the database's
/// values are sealed with, or when the database cannot be read or written.
#[instrument(skip(db))]
pub async fn verify_secret_key(db: &Database) -> Result<()> {
    let key = db.secret_key()?;
    let writer = db.writer().await?;
    let sealed = query_optional(
        &writer.conn,
        queries::definition::FIND_SECRET_KEY_CHECK.bind([CHECK_ROW.into()]),
        sealed_from_row,
    )
    .await
    .context("Failed to read the database key check")?;
    if let Some(sealed) = sealed {
        let opened = open(key, &sealed, CHECK_CONTEXT).map_err(|_| {
            anyhow!(
                "The database key does not match the key this database's secret values are \
                 sealed with. Restore the key file that was backed up with this database"
            )
        })?;
        if opened != CHECK_PLAINTEXT {
            bail!("The database key check holds an unexpected value");
        }
        return Ok(());
    }
    execute(
        &writer.conn,
        queries::definition::CREATE_SECRET_KEY_CHECK.bind([
            CHECK_ROW.into(),
            Value::Blob(seal(key, CHECK_PLAINTEXT, CHECK_CONTEXT)?),
        ]),
    )
    .await
    .context("Failed to record the database key check")?;
    tracing::info!("Recorded the database key check");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_value_opens_with_its_key_and_place() {
        let key = SecretKey::generate();
        let sealed = seal(&key, b"hunter2", "lab.ztp_server_password:abc").unwrap();
        assert_eq!(sealed[0], SEALED_VERSION);
        assert_eq!(
            sealed.len(),
            1 + NONCE_LENGTH + b"hunter2".len() + TAG_LENGTH
        );
        assert!(!sealed.windows(7).any(|window| window == b"hunter2"));
        assert_eq!(
            open(&key, &sealed, "lab.ztp_server_password:abc").unwrap(),
            b"hunter2"
        );
    }

    #[test]
    fn a_wrong_key_is_refused_clearly() {
        let sealed = seal(&SecretKey::generate(), b"hunter2", "place").unwrap();
        let error = open(&SecretKey::generate(), &sealed, "place").unwrap_err();
        assert!(
            error.to_string().contains("different database key"),
            "{error}"
        );
    }

    #[test]
    fn a_value_moved_to_another_place_does_not_open() {
        let key = SecretKey::generate();
        let sealed = seal(&key, b"hunter2", "lab.ztp_server_password:abc").unwrap();
        assert!(open(&key, &sealed, "lab.ztp_server_password:xyz").is_err());
    }

    #[test]
    fn a_tampered_value_is_refused() {
        let key = SecretKey::generate();
        let sealed = seal(&key, b"hunter2", "place").unwrap();
        for index in 1..sealed.len() {
            let mut tampered = sealed.clone();
            tampered[index] ^= 0x01;
            assert!(open(&key, &tampered, "place").is_err(), "byte {index}");
        }
        let mut version = sealed.clone();
        version[0] = 2;
        let error = open(&key, &version, "place").unwrap_err();
        assert!(error.to_string().contains("format 2"), "{error}");
        assert!(open(&key, &sealed[..20], "place").is_err());
        assert!(open(&key, &[], "place").is_err());
    }

    #[test]
    fn each_value_gets_its_own_nonce() {
        let key = SecretKey::generate();
        let first = seal(&key, b"same", "place").unwrap();
        let second = seal(&key, b"same", "place").unwrap();
        assert_ne!(
            first[1..=NONCE_LENGTH],
            second[1..=NONCE_LENGTH],
            "nonces repeat"
        );
        assert_ne!(first, second);
    }

    #[test]
    fn sealed_text_is_prefixed_hex_and_opens_only_sealed() {
        let key = SecretKey::generate();
        let sealed = seal_text(&key, "cisco", "user.device_password:abc").unwrap();
        let Value::Text(text) = Value::from(sealed) else {
            panic!("sealed text is not text");
        };
        let hex = text.strip_prefix(SEALED_TEXT_PREFIX).unwrap();
        assert!(hex.bytes().all(|byte| HEX_DIGITS.contains(&byte)), "{text}");
        assert!(!text.contains("cisco"));
        assert_eq!(
            open_text(&key, &text, "user.device_password:abc").unwrap(),
            "cisco"
        );
        for stored in ["cisco", "sealed:zz", "sealed:abc", "sealed:00"] {
            assert!(open_text(&key, stored, "place").is_err(), "{stored}");
        }
        let error = open_text(&key, "cisco", "place").unwrap_err();
        assert!(error.to_string().contains("not sealed"), "{error}");
        assert!(open_text(&key, &text, "user.device_password:other").is_err());
    }

    #[test]
    fn a_key_must_be_32_bytes_and_hides_them_when_printed() {
        assert!(SecretKey::from_bytes(&[0; 31]).is_err());
        let key = SecretKey::from_bytes(&[7; 32]).unwrap();
        assert_eq!(key.as_bytes(), &[7; 32]);
        assert_eq!(format!("{key:?}"), "SecretKey(..)");
    }

    #[tokio::test]
    async fn the_key_check_is_written_once_and_refuses_another_key() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::open(&dir.path().join("sherpa.sqlite"))
            .await
            .unwrap();
        crate::apply_schema(&db).await.unwrap();
        assert!(!secret_key_registered(&db).await.unwrap());

        let key = SecretKey::generate();
        let keyed = db.clone().with_secret_key(key.clone());
        verify_secret_key(&keyed).await.unwrap();
        assert!(secret_key_registered(&db).await.unwrap());
        verify_secret_key(&keyed).await.unwrap();

        let other = db.clone().with_secret_key(SecretKey::generate());
        let error = verify_secret_key(&other).await.unwrap_err();
        assert!(error.to_string().contains("does not match"), "{error}");

        let error = verify_secret_key(&db).await.unwrap_err();
        assert!(error.to_string().contains("not loaded"), "{error}");
    }
}
