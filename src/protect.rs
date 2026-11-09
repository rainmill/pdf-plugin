//! Password protection: a finished PDF encrypted with AES-256 (the PDF 2.0
//! standard security handler, revision 6), so it opens only with the password.

use std::collections::BTreeMap;
use std::sync::Arc;

use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::{Document, EncryptionState, EncryptionVersion, Permissions};

/// `pdf` encrypted so that `password` opens it. The owner password is random
/// and discarded, so the permissions (print, copy, fill) cannot be changed.
pub(crate) fn encrypt(pdf: &[u8], password: &str) -> Result<Vec<u8>, String> {
    let mut doc = Document::load_mem(pdf).map_err(|e| e.to_string())?;
    let mut key = [0u8; 32];
    let mut owner = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| e.to_string())?;
    getrandom::fill(&mut owner).map_err(|e| e.to_string())?;
    let owner: String = owner.iter().map(|b| format!("{b:02x}")).collect();
    let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
    let state = EncryptionState::try_from(EncryptionVersion::V5 {
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
        file_encryption_key: &key,
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: &owner,
        user_password: password,
        permissions: Permissions::PRINTABLE
            | Permissions::PRINTABLE_IN_HIGH_QUALITY
            | Permissions::COPYABLE
            | Permissions::COPYABLE_FOR_ACCESSIBILITY
            | Permissions::FILLABLE,
    })
    .map_err(|e| e.to_string())?;
    doc.encrypt(&state).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}
