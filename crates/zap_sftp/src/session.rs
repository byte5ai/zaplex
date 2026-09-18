//! SFTP session management module
//!
//! Encapsulates SSH2 connection establishment, authentication, and SFTP subsystem channel creation.
//! author: logic
//! date: 2026-05-31

use std::fs;
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use sha2::{Digest as _, Sha256};
use ssh2::{CheckResult, HostKeyType, KnownHostFileKind, MethodType};
use zeroize::Zeroizing;

use crate::error::SftpError;
use crate::sftp::Sftp;

/// Default connection timeout (10 seconds)
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

// Serialize this process's read-modify-write updates; atomic rename alone does
// not prevent two SFTP sessions from overwriting each other's newly trusted keys.
static KNOWN_HOSTS_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Authentication method
#[derive(Clone)]
pub enum AuthMethod {
    Password {
        password: Zeroizing<String>,
    },
    PublicKey {
        key_path: PathBuf,
        passphrase: Option<Zeroizing<String>>,
    },
    /// Key authentication with no key file configured for the host — the
    /// common case when the user relies on `~/.ssh/config` + `ssh-agent`, as
    /// the terminal path does (it shells out to `ssh`, which reads both).
    /// Tries the agent first, then OpenSSH's default identity files, so the
    /// file manager reaches the same hosts the terminal can.
    AgentOrDefaultKeys {
        passphrase: Option<Zeroizing<String>>,
    },
}

/// The exact host-key fingerprint approved by the user for one retry.
#[derive(Clone, Eq, PartialEq)]
pub struct HostKeyConfirmation {
    host: String,
    port: u16,
    fingerprint_sha256: String,
    replace_existing: bool,
}

impl HostKeyConfirmation {
    pub fn new(host: String, port: u16, fingerprint_sha256: String) -> Self {
        Self {
            host,
            port,
            fingerprint_sha256,
            replace_existing: false,
        }
    }

    /// Approves replacing the stored key for one endpoint with the exact key
    /// returned by the preceding handshake.
    pub fn replacement(host: String, port: u16, fingerprint_sha256: String) -> Self {
        Self {
            host,
            port,
            fingerprint_sha256,
            replace_existing: true,
        }
    }

    pub fn fingerprint_sha256(&self) -> &str {
        &self.fingerprint_sha256
    }

    fn matches_endpoint(&self, host: &str, port: u16) -> bool {
        self.host == host && self.port == port
    }

    fn permits_replacement(&self) -> bool {
        self.replace_existing
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostKeyPolicyAction {
    Accept,
    Persist,
    Replace,
}

/// SFTP session, wraps ssh2 connection
pub struct SftpSession {
    session: Arc<ssh2::Session>,
    _tcp: TcpStream,
    /// Marks whether connection was explicitly disconnected, prevents double disconnect in Drop
    disconnected: Arc<AtomicBool>,
}

impl SftpSession {
    /// Establish SSH connection with specified parameters
    ///
    /// # Parameters
    /// - `host`: server address
    /// - `port`: server port
    /// - `username`: username
    /// - `auth`: authentication method
    /// - `timeout`: optional timeout duration; None uses default 10 seconds
    pub fn connect(
        host: &str,
        port: u16,
        username: &str,
        auth: AuthMethod,
        timeout: Option<Duration>,
    ) -> Result<Self, SftpError> {
        Self::connect_with_confirmation(host, port, username, auth, timeout, None)
    }

    /// Establishes a connection after the user approved an exact SHA256 host-key fingerprint.
    ///
    /// The fingerprint is checked against the key from this new handshake before the key is
    /// persisted or any authentication method is attempted.
    pub fn connect_confirmed(
        host: &str,
        port: u16,
        username: &str,
        auth: AuthMethod,
        timeout: Option<Duration>,
        confirmation: &HostKeyConfirmation,
    ) -> Result<Self, SftpError> {
        Self::connect_with_confirmation(host, port, username, auth, timeout, Some(confirmation))
    }

    fn connect_with_confirmation(
        host: &str,
        port: u16,
        username: &str,
        auth: AuthMethod,
        timeout: Option<Duration>,
        confirmation: Option<&HostKeyConfirmation>,
    ) -> Result<Self, SftpError> {
        let effective_timeout = timeout.unwrap_or(DEFAULT_TIMEOUT);
        let addr = format!("{host}:{port}");

        // Resolve DNS via ToSocketAddrs; supports hostnames and IP addresses
        let socket_addr = addr
            .to_socket_addrs()
            .map_err(|e| SftpError::ConnectionFailed(format!("Address resolution failed: {e}")))?
            .next()
            .ok_or_else(|| {
                SftpError::ConnectionFailed(format!("DNS resolution returned no results: {addr}"))
            })?;

        // Use TCP connection with timeout
        let tcp = TcpStream::connect_timeout(&socket_addr, effective_timeout).map_err(|e| {
            if e.kind() == std::io::ErrorKind::TimedOut {
                SftpError::Timeout
            } else {
                SftpError::ConnectionFailed(format!("Failed to connect to {addr}: {e}"))
            }
        })?;

        let mut session = ssh2::Session::new().map_err(|e| {
            SftpError::ConnectionFailed(format!("Failed to create SSH session: {e}"))
        })?;

        let tcp_for_session = tcp
            .try_clone()
            .map_err(|e| SftpError::ConnectionFailed(format!("Failed to clone TCP stream: {e}")))?;
        session.set_tcp_stream(tcp_for_session);

        // Set SSH session timeout (milliseconds); affects handshake and all subsequent blocking operations
        session.set_timeout(effective_timeout.as_millis() as u32);

        let known_hosts_path = default_known_hosts_path()?;
        let known_hosts = load_known_hosts(&session, &known_hosts_path)?;
        let algorithms = preferred_host_key_algorithms(&session, &known_hosts, host, port)?;
        session.method_pref(MethodType::HostKey, &algorithms.join(","))?;

        session.handshake().map_err(|e| {
            if is_timeout_error(&e) {
                SftpError::Timeout
            } else {
                SftpError::ConnectionFailed(format!("SSH handshake failed: {e}"))
            }
        })?;

        authenticate_after_host_key_check(
            verify_host_key(&session, host, port, &known_hosts_path, confirmation),
            || authenticate(&session, username, &auth),
        )?;

        if !session.authenticated() {
            return Err(SftpError::AuthFailed("Authentication failed".into()));
        }

        // Set operation timeout (30 seconds), prevents indefinite blocking on network issues
        session.set_timeout(30_000);

        Ok(Self {
            session: Arc::new(session),
            _tcp: tcp,
            disconnected: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Get SFTP channel
    pub fn sftp(&self) -> Result<Sftp, SftpError> {
        let sftp = self.session.sftp()?;
        Ok(Sftp::new(sftp, self.session.clone()))
    }

    /// Disconnect
    pub fn disconnect(&self) -> Result<(), SftpError> {
        if self.disconnected.swap(true, Ordering::SeqCst) {
            // Already disconnected, skip
            return Ok(());
        }
        self.session.disconnect(None, "bye", None)?;
        Ok(())
    }

    /// Check if connection is alive
    pub fn is_authenticated(&self) -> bool {
        self.session.authenticated()
    }
}

fn authenticate(
    session: &ssh2::Session,
    username: &str,
    auth: &AuthMethod,
) -> Result<(), SftpError> {
    match auth {
        AuthMethod::Password { password } => session
            .userauth_password(username, password.as_str())
            .map_err(|error| auth_error("Password", error)),
        AuthMethod::PublicKey {
            key_path,
            passphrase,
        } => session
            .userauth_pubkey_file(
                username,
                None,
                key_path,
                passphrase.as_deref().map(String::as_str),
            )
            .map_err(|error| auth_error("Public key", error)),
        AuthMethod::AgentOrDefaultKeys { passphrase } => {
            authenticate_like_openssh(session, username, passphrase.as_deref().map(String::as_str))
        }
    }
}

fn auth_error(method: &str, error: ssh2::Error) -> SftpError {
    if is_timeout_error(&error) {
        SftpError::Timeout
    } else {
        SftpError::AuthFailed(format!("{method} authentication failed: {error}"))
    }
}

fn default_known_hosts_path() -> Result<PathBuf, SftpError> {
    dirs::home_dir()
        .map(|home| home.join(".ssh").join("known_hosts"))
        .ok_or_else(|| {
            SftpError::ConnectionFailed(
                "Cannot verify the SSH host key because the home directory is unavailable"
                    .to_string(),
            )
        })
}

fn load_known_hosts(session: &ssh2::Session, path: &Path) -> Result<ssh2::KnownHosts, SftpError> {
    let mut known_hosts = session.known_hosts()?;
    match fs::metadata(path) {
        Ok(_) => {
            known_hosts.read_file(path, KnownHostFileKind::OpenSSH)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(SftpError::Io(error)),
    }
    Ok(known_hosts)
}

fn check_host_endpoint(
    known_hosts: &ssh2::KnownHosts,
    host: &str,
    port: u16,
    key: &[u8],
) -> CheckResult {
    if port == 22 {
        known_hosts.check_port(host, port, key)
    } else {
        // check_port also falls back to the bare host for non-default ports.
        // A pin for port 22 must not authenticate a different endpoint.
        known_hosts.check(&format!("[{host}]:{port}"), key)
    }
}

fn public_key_algorithm(key: &[u8]) -> Option<&str> {
    let length = u32::from_be_bytes(key.get(..4)?.try_into().ok()?) as usize;
    std::str::from_utf8(key.get(4..4usize.checked_add(length)?)?).ok()
}

fn known_host_key_algorithms(
    known_hosts: &ssh2::KnownHosts,
    host: &str,
    port: u16,
) -> Result<Vec<String>, SftpError> {
    let mut algorithms = Vec::new();
    for entry in known_hosts.hosts()? {
        let Ok(key) = base64::engine::general_purpose::STANDARD.decode(entry.key()) else {
            continue;
        };
        if matches!(
            check_host_endpoint(known_hosts, host, port, &key),
            CheckResult::Match
        ) {
            if let Some(algorithm) = public_key_algorithm(&key) {
                algorithms.push(algorithm.to_string());
            }
        }
    }
    Ok(algorithms)
}

fn preferred_host_key_algorithms(
    session: &ssh2::Session,
    known_hosts: &ssh2::KnownHosts,
    host: &str,
    port: u16,
) -> Result<Vec<&'static str>, SftpError> {
    let known = known_host_key_algorithms(known_hosts, host, port)?;
    let mut supported = session.supported_algs(MethodType::HostKey)?;
    // Preserve the library's order within each group, including RSA SHA-2
    // signatures for stored ssh-rsa public keys. Never add unsupported methods.
    supported.sort_by_key(|algorithm| {
        let key_algorithm = match *algorithm {
            "rsa-sha2-256" | "rsa-sha2-512" => "ssh-rsa",
            algorithm => algorithm,
        };
        !known.iter().any(|known| known == key_algorithm)
    });
    Ok(supported)
}

fn check_known_host_key(
    session: &ssh2::Session,
    known_hosts: &ssh2::KnownHosts,
    host: &str,
    port: u16,
    key: &[u8],
) -> Result<CheckResult, SftpError> {
    let result = check_host_endpoint(known_hosts, host, port, key);
    if !matches!(result, CheckResult::Mismatch) {
        return Ok(result);
    }
    let Some(algorithm) = public_key_algorithm(key) else {
        return Ok(CheckResult::Failure);
    };
    // ssh2's check API omits the key type. Recheck only entries of the
    // negotiated algorithm so a new key type is unknown, not a rotation.
    let mut same_algorithm = session.known_hosts()?;
    for entry in known_hosts.hosts()? {
        let Ok(candidate) = base64::engine::general_purpose::STANDARD.decode(entry.key()) else {
            continue;
        };
        if public_key_algorithm(&candidate) == Some(algorithm)
            && matches!(
                check_host_endpoint(known_hosts, host, port, &candidate),
                CheckResult::Match
            )
        {
            // Identity was checked above; this temporary collection needs only
            // the exact target and its keys, not the original aliases or hashes.
            let endpoint = if port == 22 {
                host.to_string()
            } else {
                format!("[{host}]:{port}")
            };
            let encoded = base64::engine::general_purpose::STANDARD.encode(candidate);
            same_algorithm.read_str(
                &format!("{endpoint} {algorithm} {encoded}"),
                KnownHostFileKind::OpenSSH,
            )?;
        }
    }
    Ok(check_host_endpoint(&same_algorithm, host, port, key))
}

fn verify_host_key(
    session: &ssh2::Session,
    host: &str,
    port: u16,
    known_hosts_path: &Path,
    confirmation: Option<&HostKeyConfirmation>,
) -> Result<(), SftpError> {
    let (host_key, host_key_type) = session.host_key().ok_or_else(|| {
        SftpError::ConnectionFailed("SSH handshake returned no host key".to_string())
    })?;
    let fingerprint_sha256 = host_key_fingerprint_sha256(host_key);
    let key_type = host_key_type_name(host_key_type).to_string();
    let known_hosts = load_known_hosts(session, known_hosts_path)?;

    match enforce_host_key_policy(
        check_known_host_key(session, &known_hosts, host, port, host_key)?,
        host,
        port,
        fingerprint_sha256,
        key_type,
        confirmation,
    )? {
        HostKeyPolicyAction::Accept => Ok(()),
        HostKeyPolicyAction::Persist => persist_host_key(
            session,
            known_hosts_path,
            host,
            port,
            host_key,
            host_key_type,
        ),
        HostKeyPolicyAction::Replace => replace_host_key(
            session,
            known_hosts_path,
            host,
            port,
            host_key,
            host_key_type,
        ),
    }
}

fn enforce_host_key_policy(
    check_result: CheckResult,
    host: &str,
    port: u16,
    fingerprint_sha256: String,
    key_type: String,
    confirmation: Option<&HostKeyConfirmation>,
) -> Result<HostKeyPolicyAction, SftpError> {
    match check_result {
        CheckResult::Match => Ok(HostKeyPolicyAction::Accept),
        CheckResult::Mismatch => {
            if confirmation.is_some_and(|confirmation| {
                confirmation.matches_endpoint(host, port)
                    && confirmation.fingerprint_sha256() == fingerprint_sha256.as_str()
                    && confirmation.permits_replacement()
            }) {
                Ok(HostKeyPolicyAction::Replace)
            } else {
                Err(SftpError::HostKeyMismatch {
                    fingerprint_sha256,
                    key_type,
                })
            }
        }
        CheckResult::NotFound => {
            if confirmation.is_some_and(|confirmation| {
                confirmation.matches_endpoint(host, port)
                    && confirmation.fingerprint_sha256() == fingerprint_sha256.as_str()
            }) {
                Ok(HostKeyPolicyAction::Persist)
            } else if confirmation.is_some() {
                Err(SftpError::HostKeyMismatch {
                    fingerprint_sha256,
                    key_type,
                })
            } else {
                Err(SftpError::UnknownHostKey {
                    fingerprint_sha256,
                    key_type,
                })
            }
        }
        CheckResult::Failure => Err(SftpError::ConnectionFailed(
            "Failed to verify the SSH host key against known_hosts".to_string(),
        )),
    }
}

fn replace_host_key(
    session: &ssh2::Session,
    known_hosts_path: &Path,
    host: &str,
    port: u16,
    host_key: &[u8],
    host_key_type: HostKeyType,
) -> Result<(), SftpError> {
    let _write_guard = KNOWN_HOSTS_WRITE_LOCK.lock().map_err(|error| {
        SftpError::ConnectionFailed(format!("Cannot lock known_hosts for writing: {error}"))
    })?;
    let Some(algorithm) = public_key_algorithm(host_key) else {
        return Err(SftpError::ConnectionFailed(
            "Invalid SSH host key".to_string(),
        ));
    };
    let original = fs::read_to_string(known_hosts_path)?;
    let mut updated = String::new();
    let mut removed_existing_key = false;
    for line in original.split_inclusive('\n') {
        let mut fields = line.split_whitespace();
        let (Some(hosts), Some(key_type), Some(key)) =
            (fields.next(), fields.next(), fields.next())
        else {
            updated.push_str(line);
            continue;
        };
        if hosts.starts_with('#') || key_type != algorithm {
            updated.push_str(line);
            continue;
        }
        let mut remaining_hosts = Vec::new();
        for candidate_host in hosts.split(',') {
            // Match each original token independently. Iterating/removing ssh2
            // Host objects aliases hashed entries that share the same key.
            let mut candidate = session.known_hosts()?;
            candidate.read_str(
                &format!("{candidate_host} {key_type} {key}"),
                KnownHostFileKind::OpenSSH,
            )?;
            match check_host_endpoint(&candidate, host, port, host_key) {
                CheckResult::Match | CheckResult::Mismatch => removed_existing_key = true,
                CheckResult::NotFound => remaining_hosts.push(candidate_host),
                CheckResult::Failure => {
                    return Err(SftpError::ConnectionFailed(
                        "The stored SSH host key could not be checked safely".to_string(),
                    ))
                }
            }
        }
        if remaining_hosts.len() == hosts.split(',').count() {
            updated.push_str(line);
        } else if !remaining_hosts.is_empty() {
            let start = line.len() - line.trim_start().len();
            updated.push_str(&line[..start]);
            updated.push_str(&remaining_hosts.join(","));
            updated.push_str(&line[start + hosts.len()..]);
        }
    }
    if !removed_existing_key {
        return Err(SftpError::ConnectionFailed(
            "The stored SSH host key could not be replaced safely".to_string(),
        ));
    }
    append_host_key(session, &mut updated, host, port, host_key, host_key_type)?;
    write_known_hosts(known_hosts_path, &updated)
}

fn authenticate_after_host_key_check<T>(
    verification: Result<(), SftpError>,
    authenticate: impl FnOnce() -> Result<T, SftpError>,
) -> Result<T, SftpError> {
    verification?;
    authenticate()
}

fn persist_host_key(
    session: &ssh2::Session,
    known_hosts_path: &Path,
    host: &str,
    port: u16,
    host_key: &[u8],
    host_key_type: HostKeyType,
) -> Result<(), SftpError> {
    let _write_guard = KNOWN_HOSTS_WRITE_LOCK.lock().map_err(|error| {
        SftpError::ConnectionFailed(format!("Cannot lock known_hosts for writing: {error}"))
    })?;
    // Keep all original text, including OpenSSH markers libssh2 cannot round-trip.
    let mut updated = match fs::read_to_string(known_hosts_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(SftpError::Io(error)),
    };
    append_host_key(session, &mut updated, host, port, host_key, host_key_type)?;
    write_known_hosts(known_hosts_path, &updated)
}

fn append_host_key(
    session: &ssh2::Session,
    contents: &mut String,
    host: &str,
    port: u16,
    host_key: &[u8],
    host_key_type: HostKeyType,
) -> Result<(), SftpError> {
    let endpoint = if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    };
    let mut entry = session.known_hosts()?;
    entry.add(
        &endpoint,
        host_key,
        "added by Zaplex SFTP",
        host_key_type.into(),
    )?;
    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    for host in entry.hosts()? {
        contents.push_str(&entry.write_string(&host, KnownHostFileKind::OpenSSH)?);
    }
    Ok(())
}

fn write_known_hosts(path: &Path, contents: &str) -> Result<(), SftpError> {
    // Preserve an existing symlink and atomically update its target instead.
    let target = match fs::canonicalize(path) {
        Ok(target) => target,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
                return Err(SftpError::Io(error));
            }
            path.to_path_buf()
        }
        Err(error) => return Err(SftpError::Io(error)),
    };
    let parent = target.parent().ok_or_else(|| {
        SftpError::ConnectionFailed("known_hosts path has no parent directory".to_string())
    })?;
    let mut directory = fs::DirBuilder::new();
    directory.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory.create(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    match fs::metadata(&target) {
        Ok(metadata) => temporary
            .as_file()
            .set_permissions(metadata.permissions())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(SftpError::Io(error)),
    }
    temporary.write_all(contents.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&target)
        .map_err(|error| SftpError::Io(error.error))?;
    // The replacement has already committed. A filesystem that cannot flush a
    // directory must not turn a successful key update into a connection error.
    #[cfg(unix)]
    let _ = fs::File::open(parent).and_then(|directory| directory.sync_all());
    Ok(())
}

fn host_key_fingerprint_sha256(host_key: &[u8]) -> String {
    let digest = Sha256::digest(host_key);
    format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
    )
}

fn host_key_type_name(host_key_type: HostKeyType) -> &'static str {
    match host_key_type {
        HostKeyType::Unknown => "unknown",
        HostKeyType::Rsa => "RSA",
        HostKeyType::Dss => "DSA",
        HostKeyType::Ecdsa256 => "ECDSA P-256",
        HostKeyType::Ecdsa384 => "ECDSA P-384",
        HostKeyType::Ecdsa521 => "ECDSA P-521",
        HostKeyType::Ed25519 => "ED25519",
    }
}

impl Drop for SftpSession {
    fn drop(&mut self) {
        if !self.disconnected.swap(true, Ordering::SeqCst) {
            let _ = self.session.disconnect(None, "bye", None);
        }
    }
}

/// Check if ssh2 error is a timeout error
fn is_timeout_error(error: &ssh2::Error) -> bool {
    // ssh2 error code Session(-37) corresponds to LIBSSH2_ERROR_SOCKET_TIMEOUT
    error.code() == ssh2::ErrorCode::Session(-37)
}

/// What `ssh` itself does for a host with no explicit `IdentityFile`: try the
/// agent (every identity it holds, not just the first), then the default
/// identity files in `~/.ssh`. Returns the LAST error when everything fails,
/// so the message names something actionable rather than "no key path".
fn authenticate_like_openssh(
    session: &ssh2::Session,
    username: &str,
    passphrase: Option<&str>,
) -> Result<(), SftpError> {
    // 1. ssh-agent. Every identity is tried: `userauth_agent` only ever
    //    attempts the agent's FIRST key, so a user whose agent holds several
    //    keys (the normal case) would fail here while plain `ssh` succeeds.
    //    A missing SSH_AUTH_SOCK makes `connect` fail immediately.
    if let Ok(mut agent) = session.agent() {
        if agent.connect().is_ok() {
            let mut authenticated = false;
            if agent.list_identities().is_ok() {
                if let Ok(identities) = agent.identities() {
                    for identity in identities {
                        if agent.userauth(username, &identity).is_ok() && session.authenticated() {
                            authenticated = true;
                            break;
                        }
                    }
                }
            }
            // libssh2's own example disconnects before the agent is freed;
            // `Drop` only calls `libssh2_agent_free`.
            let _ = agent.disconnect();
            if authenticated {
                return Ok(());
            }
        }
    }

    // 2. Default identity files, in ssh's own preference order (ssh_config(5)
    //    IdentityFile defaults). Only files that exist are attempted, which
    //    keeps us clear of the server's MaxAuthTries in the common case.
    let home = match std::env::var("HOME") {
        Ok(h) if !h.is_empty() => PathBuf::from(h),
        _ => {
            return Err(SftpError::AuthFailed(
                "No SSH agent identity was accepted and $HOME is unset, so the \
                 default keys in ~/.ssh cannot be located. Add a key file to \
                 this host's settings."
                    .to_string(),
            ))
        }
    };
    let mut last_err: Option<String> = None;
    let mut tried_any = false;
    for name in [
        "id_rsa",
        "id_ecdsa",
        "id_ecdsa_sk",
        "id_ed25519",
        "id_ed25519_sk",
        "id_dsa",
    ] {
        let key_path = home.join(".ssh").join(name);
        if !key_path.exists() {
            continue;
        }
        tried_any = true;
        match session.userauth_pubkey_file(username, None, &key_path, passphrase) {
            Ok(()) if session.authenticated() => return Ok(()),
            Ok(()) => {}
            Err(e) if is_timeout_error(&e) => return Err(SftpError::Timeout),
            Err(e) => last_err = Some(format!("{name}: {e}")),
        }
    }

    Err(SftpError::AuthFailed(match (tried_any, last_err) {
        (true, Some(e)) => format!(
            "No SSH agent identity was accepted and none of the default keys in \
             ~/.ssh worked ({e}). Add the right key file to this host's settings."
        ),
        (true, None) => "No SSH agent identity and no default key in ~/.ssh was \
             accepted. Add the right key file to this host's settings."
            .to_string(),
        (false, _) => "No SSH agent identity was accepted and no default key \
             exists in ~/.ssh. Add a key file to this host's settings."
            .to_string(),
    }))
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "session_live_tests.rs"]
mod live_tests;
