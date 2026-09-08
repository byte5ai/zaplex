//! Implementation of the [`SecureStorage`] service for the Linux platform.

use std::cell::OnceCell;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{
    DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _,
};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use rand::RngCore;
use ring::aead;
use secret_service::{
    blocking::{Item, SecretService},
    EncryptionType,
};
use zeroize::Zeroizing;

use super::Error;

const FALLBACK_KEY_FILE: &str = ".secure-storage-key";
const FALLBACK_KEY_LEN: usize = 32;
const LEGACY_FALLBACK_KEY: &[u8] = b"zap-local-secure-storage-fallback-key";

/// Implementation of the SecureStorage service using the Secret Service API.
pub struct SecureStorage {
    /// The value to set for the "service" attribute, used to define a
    /// namespace for keys for the application.
    service_name: String,

    /// A lazily-initialized reference to the default secret collection as
    /// provided by the installed Secret Service API provider.
    collection: OnceCell<Collection>,

    /// The fallback path to a directory in case a secret collection is
    /// not available.
    fallback_dir: Option<PathBuf>,

    /// The encryption fallback key.
    encryption_key: OnceCell<aead::LessSafeKey>,
}

impl SecureStorage {
    /// Creates a new [`SecureStorage`] instance.
    ///
    /// This does not eagerly open a connection to dbus or the underlying
    /// Secret Service provider.
    pub fn new(service_name: &str) -> Self {
        Self {
            service_name: service_name.to_owned(),
            collection: OnceCell::new(),
            fallback_dir: None,
            encryption_key: OnceCell::new(),
        }
    }

    /// Creates a new [`SecureStorage`] instance with disk fallback
    ///
    /// Does the same work as [`SecureStorage::new`], as well as storing
    /// a path to a fallback directory.
    pub fn new_with_fallback(service_name: &str, fallback_dir: PathBuf) -> Self {
        Self {
            service_name: service_name.to_owned(),
            collection: OnceCell::new(),
            fallback_dir: Some(fallback_dir),
            encryption_key: OnceCell::new(),
        }
    }

    /// Returns a reference to the default secret collection, lazily
    /// instantiating the underlying service and collection reference,
    /// returning an error if the connection cannot be established or the
    /// collection cannot be opened.
    ///
    fn collection(&self) -> Result<&secret_service::blocking::Collection<'_>, Error> {
        let collection = initialize_once_on_success(&self.collection, || {
            Collection::open_default_collection().map_err(|error| {
                log::error!("Failed to acquire default Secret Service collection: {error:#}");
                error
            })
        })?;
        let collection = collection.borrow_collection();
        // Unlock failures are not cached, so a later operation can retry them.
        collection.unlock()?;
        Ok(collection)
    }

    /// Returns the installation-scoped fallback key.
    ///
    /// The protection boundary is the owning user's 0700 directory and 0600 key file. Encryption
    /// prevents accidental disclosure but cannot protect a process running as the same user.
    fn encryption_key(&self) -> Result<&aead::LessSafeKey, Error> {
        initialize_once_on_success(&self.encryption_key, || {
            self.load_or_create_encryption_key()
        })
    }

    fn load_or_create_encryption_key(&self) -> Result<aead::LessSafeKey, Error> {
        let fallback_dir = self.ensure_fallback_dir()?;
        let key_path = fallback_dir.join(FALLBACK_KEY_FILE);
        let mut generated = Zeroizing::new([0u8; FALLBACK_KEY_LEN]);
        rand::thread_rng().fill_bytes(&mut generated[..]);

        let key_bytes = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&key_path)
        {
            Ok(mut file) => {
                if let Err(error) = file
                    .write_all(&generated[..])
                    .and_then(|()| file.sync_all())
                {
                    drop(file);
                    let _ = fs::remove_file(&key_path);
                    return Err(Error::Unknown(error.into()));
                }
                Zeroizing::new(generated.to_vec())
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Zeroizing::new(self.read_private_file(&key_path, FALLBACK_KEY_LEN)?)
            }
            Err(error) => return Err(Error::Unknown(error.into())),
        };

        make_encryption_key(&key_bytes)
    }

    fn ensure_fallback_dir(&self) -> Result<&Path, Error> {
        let Some(fallback_dir) = self.fallback_dir.as_deref() else {
            return Err(Error::NotFound);
        };
        match fs::symlink_metadata(fallback_dir) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut builder = fs::DirBuilder::new();
                builder.recursive(true).mode(0o700);
                if let Err(error) = builder.create(fallback_dir) {
                    if error.kind() != std::io::ErrorKind::AlreadyExists {
                        return Err(Error::Unknown(error.into()));
                    }
                }
            }
            Err(error) => return Err(Error::Unknown(error.into())),
        }
        validate_owned_path(fallback_dir, PathKind::Directory, 0o700, true)?;
        Ok(fallback_dir)
    }

    fn read_private_file(&self, path: &Path, expected_len: usize) -> Result<Vec<u8>, Error> {
        validate_owned_path(path, PathKind::File, 0o600, true)?;
        let bytes = fs::read(path).map_err(|error| Error::Unknown(error.into()))?;
        if bytes.len() != expected_len {
            return Err(Error::Unknown(anyhow!(
                "Fallback key has invalid length: expected {expected_len}, got {}",
                bytes.len()
            )));
        }
        Ok(bytes)
    }

    /// Returns the set of attributes which should be used when interacting
    /// with a secret item that is identified by the given key.
    fn attributes_for_key<'a>(&'a self, key: &'a str) -> HashMap<&'static str, &'a str> {
        HashMap::from([
            // Ensure our keys don't conflict with ones stored by another
            // application.
            ("service", self.service_name.as_str()),
            // Specify the key for the secret.
            ("key", key),
        ])
    }

    /// Provides the given function access to a secret item with the given key
    /// in order to read or manipulate the item.
    fn with_item<T>(
        &self,
        key: &str,
        func: impl FnOnce(&Item) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let collection = self.collection()?;
        let items = collection.search_items(self.attributes_for_key(key))?;
        let Some(item) = items.first() else {
            return Err(Error::NotFound);
        };
        func(item)
    }

    fn write_secret_value(&self, key: &str, value: &str) -> Result<(), Error> {
        let collection = self.collection()?;
        // Construct a slightly more human-readable label for the secret than
        // using the key alone.
        let label = format!("{}: {key}", self.service_name);
        collection.create_item(
            &label,
            self.attributes_for_key(key),
            value.as_bytes(),
            // replace the existing key, if any
            true,
            "text/plain",
        )?;
        Ok(())
    }

    fn fallback_encrypt(&self, value: &str) -> Result<Vec<u8>, Error> {
        encrypt_with_key(self.encryption_key()?, value)
    }

    fn fallback_decrypt(&self, value: &[u8]) -> Result<String, Error> {
        decrypt_with_key(self.encryption_key()?, value)
    }

    fn legacy_fallback_decrypt(&self, value: &[u8]) -> Result<String, Error> {
        let mut legacy_key = Zeroizing::new(LEGACY_FALLBACK_KEY.to_vec());
        legacy_key.resize(FALLBACK_KEY_LEN, 0);
        let key = make_encryption_key(&legacy_key)?;
        decrypt_with_key(&key, value)
    }

    fn fallback_file(&self, key: &str) -> Result<PathBuf, Error> {
        let dir = self.ensure_fallback_dir()?;
        let filename = format!("{}-{key}", self.service_name);
        Ok(dir.join(filename))
    }

    fn write_fallback_value(&self, key: &str, value: &str) -> Result<(), Error> {
        let fallback_file = self.fallback_file(key)?;
        repair_existing_private_file(&fallback_file)?;
        let encrypted = self.fallback_encrypt(value)?;
        atomic_write_private(&fallback_file, &encrypted)
    }

    fn read_fallback_value(&self, key: &str) -> Result<String, Error> {
        let fallback_file = self.fallback_file(key)?;
        match fs::symlink_metadata(&fallback_file) {
            Ok(_) => validate_owned_path(&fallback_file, PathKind::File, 0o600, true)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::NotFound)
            }
            Err(error) => return Err(Error::Unknown(error.into())),
        }
        let data = fs::read(&fallback_file).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::NotFound
            } else {
                Error::Unknown(error.into())
            }
        })?;
        match self.fallback_decrypt(&data) {
            Ok(value) => Ok(value),
            Err(primary_error) => match self.legacy_fallback_decrypt(&data) {
                Ok(value) => {
                    self.write_fallback_value(key, &value)?;
                    Ok(value)
                }
                Err(_) => Err(primary_error),
            },
        }
    }

    fn delete_fallback_value(&self, key: &str) -> Result<(), Error> {
        let fallback_file = self.fallback_file(key)?;
        match fs::symlink_metadata(&fallback_file) {
            Ok(_) => validate_owned_path(&fallback_file, PathKind::File, 0o600, false)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::NotFound)
            }
            Err(error) => return Err(Error::Unknown(error.into())),
        }
        fs::remove_file(fallback_file).map_err(|error| match error {
            ref io_error if io_error.kind() == std::io::ErrorKind::NotFound => Error::NotFound,
            io_error => Error::Unknown(io_error.into()),
        })
    }
}

fn initialize_once_on_success<T, E>(
    cell: &OnceCell<T>,
    initialize: impl FnOnce() -> Result<T, E>,
) -> Result<&T, E> {
    if cell.get().is_none() {
        let value = initialize()?;
        let _ = cell.set(value);
    }
    Ok(cell
        .get()
        .expect("successful initialization must populate the cell"))
}

fn make_encryption_key(key_bytes: &[u8]) -> Result<aead::LessSafeKey, Error> {
    let key = aead::UnboundKey::new(&aead::AES_256_GCM, key_bytes)
        .map_err(Into::<Error>::into)
        .context("Failed to initialize fallback encryption key")?;
    Ok(aead::LessSafeKey::new(key))
}

fn encrypt_with_key(encryption_key: &aead::LessSafeKey, value: &str) -> Result<Vec<u8>, Error> {
    // Generates nonce by randomly generating numbers
    // This is not the official best way to do this, but it should
    // be fine for our purposes.
    let mut rng = rand::thread_rng();
    let mut nonce_bytes = [0u8; aead::NONCE_LEN];
    rng.fill_bytes(&mut nonce_bytes);
    let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);

    let mut data = value.as_bytes().to_vec();
    encryption_key
        .seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut data)
        .map_err(Into::<Error>::into)
        .context("Fallback encryption failed")?;

    // We serialize this to disk as the 12 byte nonce followed by the message.
    let mut output = Vec::<u8>::with_capacity(aead::NONCE_LEN + data.len());
    output.extend_from_slice(&nonce_bytes);
    output.append(&mut data);

    Ok(output)
}

fn decrypt_with_key(encryption_key: &aead::LessSafeKey, value: &[u8]) -> Result<String, Error> {
    if value.len() < aead::NONCE_LEN + 1 {
        return Err(Error::Unknown(anyhow!(
            "Attempting to decrypt too small value for fallback decryption"
        )));
    }

    // The first 12 bytes of the message are the nonce.
    let nonce_bytes = &value[0..aead::NONCE_LEN];
    let nonce = aead::Nonce::try_assume_unique_for_key(nonce_bytes)
        .map_err(Into::<Error>::into)
        .context("Failed to parse nonce for fallback decryption")?;

    // The remaining bytes in the message are the data.
    // We convert this to owned b/c the decryption happens in place.
    let mut data_bytes = value[aead::NONCE_LEN..].to_owned();
    let decrypted_length = encryption_key
        .open_in_place(nonce, aead::Aad::empty(), &mut data_bytes)
        .map_err(Into::<Error>::into)
        .context("Fallback decryption failed")?
        .len();

    // The decryption happens in place, but does not resize the vec.
    // Meanwhile, a slice referring to the decrypted data is returned.
    // We use the length of that slice to resize the currently owned Vec,
    // so it can be consumed by String::from_utf8 later on.
    data_bytes.resize(decrypted_length, 0);

    String::from_utf8(data_bytes).map_err(|err| Error::DecodeError(err.utf8_error()))
}

#[derive(Clone, Copy)]
enum PathKind {
    Directory,
    File,
}

fn validate_owned_path(
    path: &Path,
    expected_kind: PathKind,
    expected_mode: u32,
    repair_mode: bool,
) -> Result<(), Error> {
    let metadata = fs::symlink_metadata(path).map_err(|error| Error::Unknown(error.into()))?;
    let kind_matches = match expected_kind {
        PathKind::Directory => metadata.file_type().is_dir(),
        PathKind::File => metadata.file_type().is_file(),
    };
    if !kind_matches || metadata.file_type().is_symlink() {
        return Err(Error::Unknown(anyhow!(
            "Refusing insecure fallback path {}",
            path.display()
        )));
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err(Error::Unknown(anyhow!(
            "Refusing fallback path not owned by the current user: {}",
            path.display()
        )));
    }
    if metadata.permissions().mode() & 0o777 != expected_mode {
        if !repair_mode {
            return Err(Error::Unknown(anyhow!(
                "Refusing fallback path with insecure permissions: {}",
                path.display()
            )));
        }
        fs::set_permissions(path, fs::Permissions::from_mode(expected_mode))
            .map_err(|error| Error::Unknown(error.into()))?;
        let repaired = fs::symlink_metadata(path).map_err(|error| Error::Unknown(error.into()))?;
        if repaired.uid() != unsafe { libc::geteuid() }
            || repaired.permissions().mode() & 0o777 != expected_mode
        {
            return Err(Error::Unknown(anyhow!(
                "Failed to secure fallback path {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn repair_existing_private_file(path: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(_) => validate_owned_path(path, PathKind::File, 0o600, true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::Unknown(error.into())),
    }
}

fn atomic_write_private(path: &Path, data: &[u8]) -> Result<(), Error> {
    let Some(parent) = path.parent() else {
        return Err(Error::Unknown(anyhow!("Fallback file has no parent")));
    };
    let file_name = path
        .file_name()
        .ok_or_else(|| Error::Unknown(anyhow!("Fallback file has no name")))?
        .to_string_lossy();
    let mut rng = rand::thread_rng();

    for _ in 0..32 {
        let temporary = parent.join(format!(".{file_name}.{:016x}.tmp", rng.next_u64()));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(Error::Unknown(error.into())),
        };
        let write_result = file.write_all(data).and_then(|()| file.sync_all());
        drop(file);
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temporary);
            return Err(Error::Unknown(error.into()));
        }
        if let Err(error) = fs::rename(&temporary, path) {
            let _ = fs::remove_file(&temporary);
            return Err(Error::Unknown(error.into()));
        }
        validate_owned_path(path, PathKind::File, 0o600, false)?;
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
        return Ok(());
    }

    Err(Error::Unknown(anyhow!(
        "Failed to allocate a temporary fallback file"
    )))
}

impl super::SecureStorage for SecureStorage {
    fn write_value(&self, key: &str, value: &str) -> Result<(), Error> {
        let secret_result = self.write_secret_value(key, value);

        match secret_result {
            Ok(_) => {
                // If we are able to write the secret value, we attempt to delete any fallback values
                let _ = self.delete_fallback_value(key);
                Ok(())
            }
            Err(_) => self.write_fallback_value(key, value),
        }
    }

    fn read_value(&self, key: &str) -> Result<String, Error> {
        let secret_result = self.with_item(key, |item| {
            let bytes = item.get_secret()?;
            String::from_utf8(bytes).map_err(|err| Error::DecodeError(err.utf8_error()))
        });

        match secret_result {
            Ok(value) => {
                // If we are able to read the secret value, we attempt to delete any fallback values
                let _ = self.delete_fallback_value(key);
                Ok(value)
            }
            // TODO(daprahamian): We might want to filter on specific error values, rather than all errors
            Err(_) => self.read_fallback_value(key),
        }
    }

    fn remove_value(&self, key: &str) -> Result<(), Error> {
        let secret_result = self.with_item(key, |item| item.delete().map_err(Into::into));
        let fs_result = self.delete_fallback_value(key);

        // We delete both the value in the secret store and the fallback values.
        // As long as one succeeds, we consider the delete a success.
        match (secret_result, fs_result) {
            (Err(secret_err), Err(_)) => Err(secret_err),
            _ => Ok(()),
        }
    }
}

impl From<secret_service::Error> for Error {
    fn from(value: secret_service::Error) -> Self {
        // TODO(vorporeal): Check to see if we can return any more specific
        // values.
        Error::Unknown(anyhow!(value))
    }
}

impl From<ring::error::Unspecified> for Error {
    fn from(value: ring::error::Unspecified) -> Self {
        Error::Unknown(anyhow!(value))
    }
}

/// A helper structure that maintains access to the default collection.
///
/// [`secret_service::SecretService`] is a self-referential struct that leaks
/// its internal reference lifetime, which is why we use [`ouroboros`] here to
/// provide a safe API for interacting with the service and collection.
#[ouroboros::self_referencing]
struct Collection {
    /// An encrypted dbus connection to the Secret Service API provider.
    #[borrows()]
    #[covariant]
    service: SecretService<'this>,

    /// A reference to the default secret collection, which can be used to
    /// add, remove and read secrets.
    #[borrows(service)]
    #[covariant]
    collection: secret_service::blocking::Collection<'this>,
}

impl Collection {
    /// Tries to open the default secret collection via the Secret Service
    /// API.
    fn open_default_collection() -> Result<Self, Error> {
        SecretService::connect(EncryptionType::Plain)
            .and_then(|service| {
                CollectionTryBuilder {
                    service,
                    collection_builder: |service| service.get_default_collection(),
                }
                .try_build()
            })
            .map_err(Into::into)
    }
}

#[cfg(test)]
#[path = "linux_test.rs"]
mod tests;
