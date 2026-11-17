//! Passwords and permissions.
//!
//! Two tools that look opposite but share one idea: a PDF can carry an *open*
//! password, which you need to read it at all, and an *owner* password, which
//! only governs whether you may print, copy or edit. The second kind is the one
//! people meet most — a bank statement that opens fine but refuses to print.
//!
//! # What this will and won't do
//!
//! `unlock` removes protection from a document you can already open: either you
//! supply the password, or the file has owner-only restrictions and opens with
//! an empty one. That is the ordinary case and what every PDF tool does.
//!
//! It does **not** guess, brute-force or otherwise attack a password. A file
//! you cannot open stays closed, and the error says so rather than offering to
//! try harder. There is no code here that iterates over candidate passwords,
//! and that is deliberate.

use std::collections::BTreeMap;
use std::sync::Arc;

use lopdf::encryption::crypt_filters::{Aes128CryptFilter, Aes256CryptFilter, CryptFilter};
use lopdf::{Document, EncryptionState, EncryptionVersion, LoadOptions, Object, Permissions, StringFormat};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Which cipher to protect with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strength {
    /// AES-256 (PDF 2.0). The right default — anything made this decade reads it.
    #[default]
    Aes256,
    /// AES-128 (PDF 1.6). For readers too old to understand AES-256.
    Aes128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Protection {
    /// Needed to open the document at all. Leave empty for owner-only
    /// restrictions, where the file opens freely but printing and copying can
    /// be refused.
    pub open_password: Option<String>,
    /// Governs permissions and allows changing the protection later. When it is
    /// left empty the open password is used for both, because an empty owner
    /// password would let anyone lift the restrictions.
    pub owner_password: Option<String>,
    pub allow_printing: bool,
    pub allow_copying: bool,
    pub allow_modifying: bool,
    pub allow_annotating: bool,
    pub strength: Strength,
}

impl Default for Protection {
    fn default() -> Self {
        Protection {
            open_password: None,
            owner_password: None,
            allow_printing: true,
            allow_copying: true,
            allow_modifying: false,
            allow_annotating: false,
            strength: Strength::Aes256,
        }
    }
}

impl Protection {
    fn permissions(&self) -> Permissions {
        let mut p = Permissions::empty();
        if self.allow_printing {
            p |= Permissions::PRINTABLE | Permissions::PRINTABLE_IN_HIGH_QUALITY;
        }
        if self.allow_copying {
            p |= Permissions::COPYABLE;
        }
        if self.allow_modifying {
            p |= Permissions::MODIFIABLE | Permissions::ASSEMBLABLE;
        }
        if self.allow_annotating {
            p |= Permissions::ANNOTABLE | Permissions::FILLABLE;
        }
        // A reader must always be allowed to hand content to assistive
        // technology; refusing that is locking out screen-reader users, not
        // protecting anything.
        p |= Permissions::COPYABLE_FOR_ACCESSIBILITY;
        p
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityInfo {
    pub encrypted: bool,
    /// Whether a password is needed to read it at all.
    pub needs_password_to_open: bool,
    /// "AES-256", "RC4 128-bit", and so on.
    pub algorithm: String,
    pub can_print: bool,
    pub can_copy: bool,
    pub can_modify: bool,
    pub can_annotate: bool,
}

impl SecurityInfo {
    pub fn summary(&self) -> String {
        if !self.encrypted {
            return "Not protected — anyone can open, print and copy it.".into();
        }
        let mut refused = Vec::new();
        if !self.can_print {
            refused.push("printing");
        }
        if !self.can_copy {
            refused.push("copying text");
        }
        if !self.can_modify {
            refused.push("editing");
        }
        let restrictions = match refused.len() {
            0 => "no restrictions".to_string(),
            1 => format!("{} is not allowed", refused[0]),
            _ => format!("{} are not allowed", refused.join(", ")),
        };
        if self.needs_password_to_open {
            format!("{} — a password is needed to open it, and {restrictions}", self.algorithm)
        } else {
            format!("{} — it opens without a password, but {restrictions}", self.algorithm)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnlockKind {
    /// The file opened with an empty password: only permissions were restricted.
    RestrictionsOnly,
    /// The supplied password opened it.
    WithPassword,
}

/// Read a document's protection without changing it.
///
/// Note that lopdf opens a file whose *user* password is empty transparently,
/// stripping the encryption as it reads. So "is it encrypted" has to be asked
/// as "was it encrypted on disk", or every owner-only protected document would
/// report as having no protection at all.
pub fn inspect_security(bytes: &[u8]) -> Result<SecurityInfo> {
    let doc = Document::load_mem(bytes)
        .map_err(|e| Error::invalid(format!("could not read this PDF: {e}")))?;

    // Still encrypted after a no-password load means a password is required.
    let needs_password_to_open = doc.is_encrypted();
    let encrypted = needs_password_to_open || doc.was_encrypted();

    if !encrypted {
        return Ok(SecurityInfo {
            encrypted: false,
            needs_password_to_open: false,
            algorithm: "none".into(),
            can_print: true,
            can_copy: true,
            can_modify: true,
            can_annotate: true,
        });
    }

    let (version, method, permission_bits) = match doc.encryption_state.as_ref() {
        // Opened with an empty password: the dictionary is gone, but the state
        // it was decoded from is still here.
        Some(state) => (
            state.version(),
            state
                .crypt_filters()
                .values()
                .next()
                .map(|f| String::from_utf8_lossy(f.method()).to_string()),
            state.permissions().bits() as u32,
        ),
        // Still locked: read it straight out of the encryption dictionary.
        None => {
            let encrypt = doc.get_encrypted().map_err(|_| {
                Error::invalid("this PDF says it is protected but has no encryption data")
            })?;
            let version = encrypt.get(b"V").and_then(|o| o.as_i64()).unwrap_or(0);
            let bits = encrypt.get(b"P").and_then(|o| o.as_i64()).unwrap_or(-1) as i32 as u32;
            (version, crypt_filter_method(&doc), bits)
        }
    };

    let allowed = |bit: u32| permission_bits & (1 << bit) != 0;
    Ok(SecurityInfo {
        encrypted: true,
        needs_password_to_open,
        algorithm: describe_algorithm(version, method.as_deref()),
        can_print: allowed(2),
        can_modify: allowed(3),
        can_copy: allowed(4),
        can_annotate: allowed(5),
    })
}

fn describe_algorithm(version: i64, method: Option<&str>) -> String {
    match version {
        1 => "RC4 40-bit".into(),
        2 => "RC4 128-bit".into(),
        4 => match method {
            Some("AESV2") => "AES-128".into(),
            Some("V2") => "RC4 128-bit".into(),
            _ => "AES-128".into(),
        },
        5 => "AES-256".into(),
        other => format!("version {other}"),
    }
}

fn crypt_filter_method(doc: &Document) -> Option<String> {
    let encrypt = doc.get_encrypted().ok()?;
    let filters = encrypt.get(b"CF").ok()?.as_dict().ok()?;
    let std = filters.get(b"StdCF").ok()?.as_dict().ok()?;
    let method = std.get(b"CFM").ok()?.as_name().ok()?;
    Some(String::from_utf8_lossy(method).to_string())
}

/// Add a password and permissions to a document.
pub fn protect(bytes: &[u8], settings: &Protection) -> Result<Vec<u8>> {
    let mut doc = Document::load_mem(bytes)
        .map_err(|e| Error::invalid(format!("could not read this PDF: {e}")))?;
    if doc.is_encrypted() {
        return Err(Error::invalid(
            "this PDF is already protected — unlock it first, then protect it again",
        ));
    }

    let open = settings.open_password.as_deref().unwrap_or("").to_string();
    let owner = match settings.owner_password.as_deref() {
        Some(p) if !p.is_empty() => p.to_string(),
        // An empty owner password would let anyone lift the restrictions,
        // which would make setting them pointless.
        _ => open.clone(),
    };
    if open.is_empty() && owner.is_empty() {
        return Err(Error::invalid(
            "set a password — either one to open the document, or one to control printing and copying",
        ));
    }

    // Revisions 3 and 4 mix the file identifier into the key, so a document
    // without one cannot be encrypted at all. Every PDF should have an ID.
    if doc.trailer.get(b"ID").is_err() {
        let mut id = [0u8; 16];
        rand::rng().fill_bytes(&mut id);
        let value = Object::String(id.to_vec(), StringFormat::Hexadecimal);
        doc.trailer.set("ID", Object::Array(vec![value.clone(), value]));
    }

    let permissions = settings.permissions();
    let state = match settings.strength {
        Strength::Aes256 => {
            let mut key = [0u8; 32];
            rand::rng().fill_bytes(&mut key);
            let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
            EncryptionState::try_from(EncryptionVersion::V5 {
                encrypt_metadata: true,
                crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
                file_encryption_key: &key,
                stream_filter: b"StdCF".to_vec(),
                string_filter: b"StdCF".to_vec(),
                owner_password: &owner,
                user_password: &open,
                permissions,
            })
        }
        Strength::Aes128 => {
            let filter: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);
            EncryptionState::try_from(EncryptionVersion::V4 {
                document: &doc,
                encrypt_metadata: true,
                crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
                stream_filter: b"StdCF".to_vec(),
                string_filter: b"StdCF".to_vec(),
                owner_password: &owner,
                user_password: &open,
                permissions,
            })
        }
    }
    .map_err(|e| Error::other(format!("could not set up encryption: {e}")))?;

    doc.encrypt(&state).map_err(|e| Error::other(format!("could not protect the file: {e}")))?;

    // Deliberately a plain save: packing objects into object streams after they
    // have each been encrypted would encrypt the container a second time.
    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| Error::other(format!("could not write the protected PDF: {e}")))?;
    Ok(out)
}

/// Remove protection from a document you can open.
///
/// Pass the password if the file needs one. Pass `None` for a file that opens
/// freely but restricts printing or copying.
pub fn unlock(bytes: &[u8], password: Option<&str>) -> Result<(Vec<u8>, UnlockKind)> {
    // Must be loaded *with* the password: a no-password load of a protected
    // file leaves every object encrypted, and decrypting afterwards operates on
    // a document that was never read properly.
    let candidate = password.unwrap_or("");
    let mut doc = Document::load_mem_with_options(bytes, LoadOptions::with_password(candidate))
        .map_err(|_| wrong_password(candidate))?;

    if !doc.is_encrypted() && !doc.was_encrypted() {
        return Err(Error::invalid("this PDF isn't protected — there is nothing to remove"));
    }
    if doc.is_encrypted() {
        // Loaded, but the password didn't open it.
        return Err(wrong_password(candidate));
    }

    // Decrypting leaves the old state behind for incremental saves; clear it so
    // nothing can carry the encryption forward.
    doc.encryption_state = None;

    let kind =
        if candidate.is_empty() { UnlockKind::RestrictionsOnly } else { UnlockKind::WithPassword };
    let bytes = super::lossless::save_compact(&mut doc)?;
    Ok((bytes, kind))
}

/// The same refusal either way, and it states the boundary: Kitbox removes
/// protection from documents you can open, and does not attack the ones you
/// cannot.
fn wrong_password(candidate: &str) -> Error {
    if candidate.is_empty() {
        Error::invalid(
            "this PDF needs its password to open. Enter it and try again — Kitbox does not guess passwords.",
        )
    } else {
        Error::invalid(
            "that password didn't open the document. Check it and try again — Kitbox does not guess passwords.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    fn plain() -> Vec<u8> {
        testkit::pdf_scan(3, 200, 260, 70)
    }

    fn pages(bytes: &[u8], password: Option<&str>) -> usize {
        let doc = match password {
            Some(p) => Document::load_mem_with_options(bytes, LoadOptions::with_password(p)).unwrap(),
            None => Document::load_mem(bytes).unwrap(),
        };
        doc.page_iter().count()
    }

    #[test]
    fn protecting_then_unlocking_round_trips() {
        let src = plain();
        let settings = Protection {
            open_password: Some("correct horse".into()),
            ..Default::default()
        };
        let locked = protect(&src, &settings).unwrap();

        let info = inspect_security(&locked).unwrap();
        assert!(info.encrypted);
        assert!(info.needs_password_to_open);
        assert_eq!(info.algorithm, "AES-256");

        let (opened, kind) = unlock(&locked, Some("correct horse")).unwrap();
        assert_eq!(kind, UnlockKind::WithPassword);
        assert!(!inspect_security(&opened).unwrap().encrypted);
        assert_eq!(pages(&opened, None), 3, "every page survives the round trip");
    }

    #[test]
    fn aes128_also_round_trips() {
        let src = plain();
        let settings = Protection {
            open_password: Some("legacy".into()),
            strength: Strength::Aes128,
            ..Default::default()
        };
        let locked = protect(&src, &settings).unwrap();
        assert_eq!(inspect_security(&locked).unwrap().algorithm, "AES-128");
        let (opened, _) = unlock(&locked, Some("legacy")).unwrap();
        assert_eq!(pages(&opened, None), 3);
    }

    #[test]
    fn the_wrong_password_is_refused_clearly() {
        let locked = protect(
            &plain(),
            &Protection { open_password: Some("right".into()), ..Default::default() },
        )
        .unwrap();

        let err = unlock(&locked, Some("wrong")).unwrap_err().to_string();
        assert!(err.contains("didn't open"), "{err}");
        assert!(err.contains("does not guess"), "should state the boundary: {err}");

        // And with no password at all, when one is required.
        let err = unlock(&locked, None).unwrap_err().to_string();
        assert!(err.contains("needs its password"), "{err}");
    }

    #[test]
    fn owner_only_protection_opens_freely_but_restricts() {
        // No open password: the document reads, but printing is refused.
        let settings = Protection {
            open_password: None,
            owner_password: Some("owner".into()),
            allow_printing: false,
            allow_copying: false,
            ..Default::default()
        };
        let locked = protect(&plain(), &settings).unwrap();

        let info = inspect_security(&locked).unwrap();
        assert!(info.encrypted);
        assert!(!info.needs_password_to_open, "an empty password should open it");
        assert!(!info.can_print);
        assert!(!info.can_copy);
        assert!(info.summary().contains("opens without a password"), "{}", info.summary());

        // Which means it can be unlocked without knowing anything.
        let (opened, kind) = unlock(&locked, None).unwrap();
        assert_eq!(kind, UnlockKind::RestrictionsOnly);
        assert!(!inspect_security(&opened).unwrap().encrypted);
        assert_eq!(pages(&opened, None), 3);
    }

    #[test]
    fn permissions_are_recorded_as_asked() {
        let settings = Protection {
            open_password: Some("pw".into()),
            allow_printing: true,
            allow_copying: false,
            allow_modifying: false,
            allow_annotating: true,
            ..Default::default()
        };
        let locked = protect(&plain(), &settings).unwrap();
        let info = inspect_security(&locked).unwrap();
        assert!(info.can_print);
        assert!(!info.can_copy);
        assert!(!info.can_modify);
        assert!(info.can_annotate);
    }

    #[test]
    fn an_unprotected_file_reports_as_such() {
        let info = inspect_security(&plain()).unwrap();
        assert!(!info.encrypted);
        assert!(info.summary().starts_with("Not protected"));
        assert!(unlock(&plain(), None).is_err(), "nothing to unlock");
    }

    #[test]
    fn protecting_twice_is_refused() {
        let locked = protect(
            &plain(),
            &Protection { open_password: Some("a".into()), ..Default::default() },
        )
        .unwrap();
        let err = protect(
            &locked,
            &Protection { open_password: Some("b".into()), ..Default::default() },
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("already protected"), "{err}");
    }

    #[test]
    fn a_password_is_required_to_protect_anything() {
        let err = protect(&plain(), &Protection::default()).unwrap_err().to_string();
        assert!(err.contains("set a password"), "{err}");
    }

    #[test]
    fn the_owner_password_falls_back_to_the_open_password() {
        // With only an open password set, an empty owner password must not
        // work — otherwise the restrictions could be lifted by anyone.
        let locked = protect(
            &plain(),
            &Protection {
                open_password: Some("shared".into()),
                owner_password: None,
                allow_printing: false,
                ..Default::default()
            },
        )
        .unwrap();
        let doc = Document::load_mem(&locked).unwrap();
        assert!(doc.authenticate_owner_password("").is_err(), "empty owner password must fail");
        assert!(doc.authenticate_owner_password("shared").is_ok());
    }

    #[test]
    fn accessibility_extraction_is_never_refused() {
        let locked = protect(
            &plain(),
            &Protection {
                open_password: Some("pw".into()),
                allow_copying: false,
                ..Default::default()
            },
        )
        .unwrap();
        let doc = Document::load_mem(&locked).unwrap();
        let p = doc.get_encrypted().unwrap().get(b"P").unwrap().as_i64().unwrap() as i32 as u32;
        assert!(p & (1 << 9) != 0, "screen readers must still be allowed to extract text");
    }
}
