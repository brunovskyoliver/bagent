use chacha20poly1305::{aead::AeadInPlace, KeyInit, XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use rand_core::{OsRng, RngCore};
use secrecy::{ExposeSecret, SecretVec};
use sha2::Sha256;
use std::{
    collections::BTreeSet,
    fmt,
    sync::{Arc, Mutex},
};
use zeroize::Zeroize;

const FORMAT_VERSION: u8 = 1;
const ENCRYPTION_KEY_VERSION: u32 = 1;
const HMAC_KEY_VERSION: u32 = 1;
const KEY_SIZE: usize = 32;
const NONCE_SIZE: usize = 24;
const SERVICE: &str = "sk.bagent.app.reference-resolution";
const ENCRYPTION_ACCOUNT: &str = "ledger-encryption.v1";
const HMAC_ACCOUNT: &str = "ledger-hmac.v1";

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CryptoFault {
    KeyUnavailable,
    KeyProvider,
    UnknownVersion,
    MalformedCiphertext,
    AuthenticationFailed,
}

#[derive(Clone)]
pub(super) struct AadBinding {
    row_id: String,
    session_id: String,
    field_purpose: String,
    turn_id: Option<String>,
    referent_id: Option<String>,
}

impl AadBinding {
    pub(super) fn new(
        row_id: impl Into<String>,
        session_id: impl Into<String>,
        field_purpose: impl Into<String>,
    ) -> Self {
        Self {
            row_id: row_id.into(),
            session_id: session_id.into(),
            field_purpose: field_purpose.into(),
            turn_id: None,
            referent_id: None,
        }
    }

    pub(super) fn with_turn(mut self, turn_id: impl Into<String>) -> Self {
        self.turn_id = Some(turn_id.into());
        self
    }

    pub(super) fn with_referent(mut self, referent_id: impl Into<String>) -> Self {
        self.referent_id = Some(referent_id.into());
        self
    }

    fn encode(&self, key_version: u32) -> Vec<u8> {
        let mut aad = Vec::with_capacity(128);
        aad.extend_from_slice(b"bagent/reference-resolution/aad/v1\0");
        aad.extend_from_slice(&1_u32.to_be_bytes());
        push_part(&mut aad, &self.field_purpose);
        push_part(&mut aad, &self.row_id);
        push_part(&mut aad, &self.session_id);
        push_part(&mut aad, self.turn_id.as_deref().unwrap_or(""));
        push_part(&mut aad, self.referent_id.as_deref().unwrap_or(""));
        aad.extend_from_slice(&key_version.to_be_bytes());
        aad
    }
}

fn push_part(buffer: &mut Vec<u8>, value: &str) {
    buffer.extend_from_slice(&(value.len() as u32).to_be_bytes());
    buffer.extend_from_slice(value.as_bytes());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum KeyKind {
    Encryption,
    Hmac,
}

pub(super) trait KeyProvider: Send + Sync {
    fn load(&self, kind: KeyKind, version: u32) -> Result<Option<Vec<u8>>, CryptoFault>;
    fn create(&self, kind: KeyKind, version: u32) -> Result<Vec<u8>, CryptoFault>;
}

pub(super) trait NonceProvider: Send + Sync {
    fn fill(&self, nonce: &mut [u8; NONCE_SIZE]);
}

struct OsNonceProvider;

impl NonceProvider for OsNonceProvider {
    fn fill(&self, nonce: &mut [u8; NONCE_SIZE]) {
        OsRng.fill_bytes(nonce);
    }
}

struct LoadedKeys {
    encryption: SecretVec<u8>,
    hmac: SecretVec<u8>,
}

pub(super) struct CryptoCustody {
    provider: Arc<dyn KeyProvider>,
    nonce_provider: Arc<dyn NonceProvider>,
    loaded: Mutex<Option<LoadedKeys>>,
}

impl fmt::Debug for CryptoCustody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CryptoCustody(<lazy>)")
    }
}

impl CryptoCustody {
    pub(super) fn production() -> Self {
        Self::with_provider(KeychainKeyProvider)
    }

    pub(super) fn with_provider<P>(provider: P) -> Self
    where
        P: KeyProvider + 'static,
    {
        Self::with_providers(provider, OsNonceProvider)
    }

    pub(super) fn with_providers<P, N>(provider: P, nonce_provider: N) -> Self
    where
        P: KeyProvider + 'static,
        N: NonceProvider + 'static,
    {
        Self {
            provider: Arc::new(provider),
            nonce_provider: Arc::new(nonce_provider),
            loaded: Mutex::new(None),
        }
    }

    pub(super) fn ensure_for_database(
        &self,
        persisted_versions: &BTreeSet<(u32, u32)>,
    ) -> Result<(), CryptoFault> {
        let has_rows = !persisted_versions.is_empty();
        if persisted_versions.iter().any(|(encryption, hmac)| {
            *encryption != ENCRYPTION_KEY_VERSION || *hmac != HMAC_KEY_VERSION
        }) {
            return Err(CryptoFault::UnknownVersion);
        }

        let mut loaded = self.loaded.lock().map_err(|_| CryptoFault::KeyProvider)?;
        if loaded.is_some() {
            return Ok(());
        }

        let encryption = load_or_create(
            self.provider.as_ref(),
            KeyKind::Encryption,
            ENCRYPTION_KEY_VERSION,
            has_rows,
        )?;
        let hmac = load_or_create(
            self.provider.as_ref(),
            KeyKind::Hmac,
            HMAC_KEY_VERSION,
            has_rows,
        )?;
        if encryption.len() != KEY_SIZE || hmac.len() != KEY_SIZE {
            return Err(CryptoFault::KeyUnavailable);
        }
        *loaded = Some(LoadedKeys {
            encryption: SecretVec::new(encryption),
            hmac: SecretVec::new(hmac),
        });
        Ok(())
    }

    pub(super) fn encrypt(
        &self,
        binding: &AadBinding,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, CryptoFault> {
        self.ensure_for_database(&BTreeSet::new())?;
        let loaded = self.loaded.lock().map_err(|_| CryptoFault::KeyProvider)?;
        let keys = loaded.as_ref().ok_or(CryptoFault::KeyUnavailable)?;
        let key = keys.encryption.expose_secret();
        let cipher =
            XChaCha20Poly1305::new_from_slice(key).map_err(|_| CryptoFault::KeyUnavailable)?;
        let mut nonce = [0_u8; NONCE_SIZE];
        self.nonce_provider.fill(&mut nonce);
        let mut ciphertext = plaintext.to_vec();
        let nonce_ref = XNonce::from_slice(&nonce);
        cipher
            .encrypt_in_place(
                nonce_ref,
                &binding.encode(ENCRYPTION_KEY_VERSION),
                &mut ciphertext,
            )
            .map_err(|_| CryptoFault::AuthenticationFailed)?;
        let mut encoded = Vec::with_capacity(1 + 4 + NONCE_SIZE + ciphertext.len());
        encoded.push(FORMAT_VERSION);
        encoded.extend_from_slice(&ENCRYPTION_KEY_VERSION.to_be_bytes());
        encoded.extend_from_slice(&nonce);
        encoded.append(&mut ciphertext);
        nonce.zeroize();
        Ok(encoded)
    }

    pub(super) fn decrypt(
        &self,
        binding: &AadBinding,
        encoded: &[u8],
    ) -> Result<SecretVec<u8>, CryptoFault> {
        if encoded.len() < 1 + 4 + NONCE_SIZE + 16 {
            return Err(CryptoFault::MalformedCiphertext);
        }
        if encoded[0] != FORMAT_VERSION {
            return Err(CryptoFault::UnknownVersion);
        }
        let key_version = u32::from_be_bytes(
            encoded[1..5]
                .try_into()
                .map_err(|_| CryptoFault::MalformedCiphertext)?,
        );
        if key_version != ENCRYPTION_KEY_VERSION {
            return Err(CryptoFault::UnknownVersion);
        }
        self.ensure_for_database(&BTreeSet::from([(key_version, HMAC_KEY_VERSION)]))?;
        let loaded = self.loaded.lock().map_err(|_| CryptoFault::KeyProvider)?;
        let keys = loaded.as_ref().ok_or(CryptoFault::KeyUnavailable)?;
        let cipher = XChaCha20Poly1305::new_from_slice(keys.encryption.expose_secret())
            .map_err(|_| CryptoFault::KeyUnavailable)?;
        let nonce = XNonce::from_slice(&encoded[5..5 + NONCE_SIZE]);
        let mut plaintext = encoded[5 + NONCE_SIZE..].to_vec();
        if cipher
            .decrypt_in_place(nonce, &binding.encode(key_version), &mut plaintext)
            .is_err()
        {
            plaintext.zeroize();
            return Err(CryptoFault::AuthenticationFailed);
        }
        Ok(SecretVec::new(plaintext))
    }

    pub(super) fn hmac(
        &self,
        binding: &AadBinding,
        normalization_version: u32,
        normalized: &[u8],
    ) -> Result<[u8; 32], CryptoFault> {
        self.ensure_for_database(&BTreeSet::new())?;
        let loaded = self.loaded.lock().map_err(|_| CryptoFault::KeyProvider)?;
        let keys = loaded.as_ref().ok_or(CryptoFault::KeyUnavailable)?;
        let mut input = hmac_input(binding, normalization_version, normalized);
        let mut mac = <HmacSha256 as Mac>::new_from_slice(keys.hmac.expose_secret())
            .map_err(|_| CryptoFault::KeyUnavailable)?;
        mac.update(&input);
        input.zeroize();
        let output = mac.finalize().into_bytes();
        let mut digest = [0_u8; 32];
        digest.copy_from_slice(&output);
        Ok(digest)
    }

    pub(super) fn verify_hmac(
        &self,
        binding: &AadBinding,
        normalization_version: u32,
        normalized: &[u8],
        expected: &[u8],
    ) -> Result<(), CryptoFault> {
        self.ensure_for_database(&BTreeSet::new())?;
        let loaded = self.loaded.lock().map_err(|_| CryptoFault::KeyProvider)?;
        let keys = loaded.as_ref().ok_or(CryptoFault::KeyUnavailable)?;
        let mut input = hmac_input(binding, normalization_version, normalized);
        let mut mac = <HmacSha256 as Mac>::new_from_slice(keys.hmac.expose_secret())
            .map_err(|_| CryptoFault::KeyUnavailable)?;
        mac.update(&input);
        input.zeroize();
        mac.verify_slice(expected)
            .map_err(|_| CryptoFault::AuthenticationFailed)
    }
}

fn hmac_input(binding: &AadBinding, normalization_version: u32, normalized: &[u8]) -> Vec<u8> {
    let mut input = binding.encode(HMAC_KEY_VERSION);
    input.extend_from_slice(b"bagent/reference-resolution/hmac/v1\0");
    input.extend_from_slice(&normalization_version.to_be_bytes());
    input.extend_from_slice(&(normalized.len() as u64).to_be_bytes());
    input.extend_from_slice(normalized);
    input
}

fn load_or_create(
    provider: &dyn KeyProvider,
    kind: KeyKind,
    version: u32,
    has_rows: bool,
) -> Result<Vec<u8>, CryptoFault> {
    match provider.load(kind, version)? {
        Some(key) => Ok(key),
        None if has_rows => Err(CryptoFault::KeyUnavailable),
        None => provider.create(kind, version),
    }
}

struct KeychainKeyProvider;

#[cfg(target_os = "macos")]
impl KeyProvider for KeychainKeyProvider {
    fn load(&self, kind: KeyKind, _version: u32) -> Result<Option<Vec<u8>>, CryptoFault> {
        use security_framework::passwords::generic_password;
        let account = match kind {
            KeyKind::Encryption => ENCRYPTION_ACCOUNT,
            KeyKind::Hmac => HMAC_ACCOUNT,
        };
        match generic_password(
            security_framework::passwords::PasswordOptions::new_generic_password(SERVICE, account),
        ) {
            Ok(key) => Ok(Some(key)),
            Err(error) if error.code() == -25300 => Ok(None),
            Err(_) => Err(CryptoFault::KeyProvider),
        }
    }

    fn create(&self, kind: KeyKind, _version: u32) -> Result<Vec<u8>, CryptoFault> {
        use security_framework::{
            access_control::{ProtectionMode, SecAccessControl},
            passwords::{set_generic_password_options, PasswordOptions},
        };
        let account = match kind {
            KeyKind::Encryption => ENCRYPTION_ACCOUNT,
            KeyKind::Hmac => HMAC_ACCOUNT,
        };
        let mut key = vec![0_u8; KEY_SIZE];
        OsRng.fill_bytes(&mut key);
        let access = SecAccessControl::create_with_protection(
            Some(ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly),
            0,
        )
        .map_err(|_| CryptoFault::KeyProvider)?;
        let mut options = PasswordOptions::new_generic_password(SERVICE, account);
        options.set_access_control(access);
        set_generic_password_options(&key, options).map_err(|_| CryptoFault::KeyProvider)?;
        Ok(key)
    }
}

#[cfg(not(target_os = "macos"))]
impl KeyProvider for KeychainKeyProvider {
    fn load(&self, _kind: KeyKind, _version: u32) -> Result<Option<Vec<u8>>, CryptoFault> {
        Err(CryptoFault::KeyProvider)
    }

    fn create(&self, _kind: KeyKind, _version: u32) -> Result<Vec<u8>, CryptoFault> {
        Err(CryptoFault::KeyProvider)
    }
}

#[cfg(test)]
pub(super) struct FakeKeyProvider {
    encryption: Mutex<Option<Vec<u8>>>,
    hmac: Mutex<Option<Vec<u8>>>,
}

#[cfg(test)]
pub(super) struct FakeNonceProvider {
    value: u8,
}

#[cfg(test)]
impl FakeNonceProvider {
    pub(super) const fn new(value: u8) -> Self {
        Self { value }
    }
}

#[cfg(test)]
impl NonceProvider for FakeNonceProvider {
    fn fill(&self, nonce: &mut [u8; NONCE_SIZE]) {
        nonce.fill(self.value);
    }
}

#[cfg(test)]
impl FakeKeyProvider {
    pub(super) fn deterministic() -> Self {
        Self {
            encryption: Mutex::new(Some(vec![0x11; KEY_SIZE])),
            hmac: Mutex::new(Some(vec![0x22; KEY_SIZE])),
        }
    }

    pub(super) fn from_keys(encryption: [u8; KEY_SIZE], hmac: [u8; KEY_SIZE]) -> Self {
        Self {
            encryption: Mutex::new(Some(encryption.to_vec())),
            hmac: Mutex::new(Some(hmac.to_vec())),
        }
    }

    pub(super) fn missing() -> Self {
        Self {
            encryption: Mutex::new(None),
            hmac: Mutex::new(None),
        }
    }
}

#[cfg(test)]
impl KeyProvider for FakeKeyProvider {
    fn load(&self, kind: KeyKind, _version: u32) -> Result<Option<Vec<u8>>, CryptoFault> {
        let value = match kind {
            KeyKind::Encryption => self.encryption.lock(),
            KeyKind::Hmac => self.hmac.lock(),
        }
        .map_err(|_| CryptoFault::KeyProvider)?;
        Ok(value.clone())
    }

    fn create(&self, kind: KeyKind, _version: u32) -> Result<Vec<u8>, CryptoFault> {
        let mut key = vec![0_u8; KEY_SIZE];
        OsRng.fill_bytes(&mut key);
        let target = match kind {
            KeyKind::Encryption => &self.encryption,
            KeyKind::Hmac => &self.hmac,
        };
        *target.lock().map_err(|_| CryptoFault::KeyProvider)? = Some(key.clone());
        Ok(key)
    }
}
