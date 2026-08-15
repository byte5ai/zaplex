//! This module implements IPC transport on top of the `interprocess` crate, which uses Unix Domain
//! Sockets on Unix platforms and named pipes on Windows under the hood.
use async_compat::CompatExt as _;
use futures::{AsyncRead, AsyncWrite};

use crate::ConnectionAddress;

pub(crate) mod client {
    use super::*;
    use crate::client::{ClientError, InitializationError, Result};
    use interprocess::local_socket::tokio::LocalSocketStream;

    /// Returns a tuple containing structs for reading and writing to a local socket, which is the
    /// underlying IPC transport for native (non-wasm) platforms.
    pub async fn connect_client(
        connection_address: ConnectionAddress,
    ) -> Result<(impl AsyncRead + Unpin, impl AsyncWrite + Unpin)> {
        let stream = LocalSocketStream::connect(connection_address.0.as_str())
            .compat()
            .await
            .map_err(|e| ClientError::Initialization(InitializationError::Io(e)))?;
        Ok(stream.into_split())
    }
}

pub(crate) mod server {
    use super::*;
    use crate::server::{InitializationError, Result, ServerError};
    use interprocess::local_socket::tokio::{LocalSocketListener, LocalSocketStream};
    #[cfg(unix)]
    use std::fs::{self, DirBuilder};
    #[cfg(unix)]
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
    #[cfg(unix)]
    use std::path::{Path, PathBuf};

    pub struct ConnectionImpl {
        stream: LocalSocketStream,
    }

    impl ConnectionImpl {
        pub fn into_split(self) -> (impl AsyncRead + Unpin, impl AsyncWrite + Unpin) {
            self.stream.into_split()
        }
    }

    pub struct ConnectionListenerImpl {
        listener: LocalSocketListener,
        #[cfg(unix)]
        cleanup_path: Option<PathBuf>,
        #[cfg(unix)]
        cleanup_parent: Option<PathBuf>,
    }

    impl ConnectionListenerImpl {
        pub fn new(connection_address: ConnectionAddress) -> Result<Self> {
            #[cfg(unix)]
            let cleanup_path = filesystem_socket_path(&connection_address);
            #[cfg(unix)]
            let cleanup_parent = match cleanup_path.as_deref() {
                Some(path) => secure_socket_parent(path)
                    .map_err(|error| ServerError::Initialization(InitializationError::Io(error)))?,
                None => None,
            };
            let listener_result = warpui::r#async::block_on(
                async move { LocalSocketListener::bind(connection_address.to_string()) }.compat(),
            );
            let listener = match listener_result {
                Ok(listener) => listener,
                Err(error) => {
                    #[cfg(unix)]
                    if let Some(parent) = cleanup_parent.as_ref() {
                        let _ = fs::remove_dir(parent);
                    }
                    return Err(ServerError::Initialization(InitializationError::Io(error)));
                }
            };
            #[cfg(unix)]
            if let Some(path) = cleanup_path.as_ref() {
                if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
                    drop(listener);
                    let _ = fs::remove_file(path);
                    if let Some(parent) = cleanup_parent.as_ref() {
                        let _ = fs::remove_dir(parent);
                    }
                    return Err(ServerError::Initialization(InitializationError::Io(error)));
                }
            }
            Ok(Self {
                listener,
                #[cfg(unix)]
                cleanup_path,
                #[cfg(unix)]
                cleanup_parent,
            })
        }

        pub async fn accept_connection(&self) -> Result<ConnectionImpl> {
            self.listener
                .accept()
                .compat()
                .await
                .map(|stream| ConnectionImpl { stream })
                .map_err(ServerError::AcceptConnection)
        }
    }

    #[cfg(unix)]
    impl Drop for ConnectionListenerImpl {
        fn drop(&mut self) {
            if let Some(path) = self.cleanup_path.as_ref() {
                let _ = fs::remove_file(path);
            }
            if let Some(parent) = self.cleanup_parent.as_ref() {
                let _ = fs::remove_dir(parent);
            }
        }
    }

    #[cfg(unix)]
    fn filesystem_socket_path(connection_address: &ConnectionAddress) -> Option<PathBuf> {
        (!connection_address.0.starts_with('@')).then(|| PathBuf::from(&connection_address.0))
    }

    #[cfg(unix)]
    fn secure_socket_parent(socket_path: &Path) -> std::io::Result<Option<PathBuf>> {
        let parent = socket_path.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "filesystem socket path has no parent directory",
            )
        })?;
        match fs::metadata(parent) {
            Ok(metadata) => {
                if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        format!(
                            "filesystem socket parent {} must be an owner-only directory",
                            parent.display()
                        ),
                    ));
                }
                Ok(None)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut builder = DirBuilder::new();
                builder.mode(0o700);
                builder.create(parent)?;
                Ok(Some(parent.to_path_buf()))
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(all(test, unix))]
#[path = "native_tests.rs"]
mod tests;
