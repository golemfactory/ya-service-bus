use std::fs;
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::Path;
use tokio::net::UnixStream;

#[derive(Clone, Copy)]
struct SocketIdentity {
    device: u64,
    inode: u64,
    uid: u32,
}

impl SocketIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            uid: metadata.uid(),
        }
    }

    fn matches(self, metadata: &fs::Metadata) -> bool {
        self.device == metadata.dev() && self.inode == metadata.ino() && self.uid == metadata.uid()
    }
}

fn error(kind: io::ErrorKind, path: &Path, message: &str) -> io::Error {
    io::Error::new(kind, format!("{message}: {}", path.display()))
}

fn inspect_owned_socket(path: &Path) -> io::Result<(fs::Metadata, SocketIdentity)> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_socket() {
        return Err(error(
            io::ErrorKind::AlreadyExists,
            path,
            "refusing to remove GSB path because it is a symlink or not a Unix socket",
        ));
    }

    let effective_uid = unsafe { libc::geteuid() };
    if metadata.uid() != effective_uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "refusing to remove GSB socket {} owned by uid {}, expected effective uid {}",
                path.display(),
                metadata.uid(),
                effective_uid
            ),
        ));
    }

    let identity = SocketIdentity::from_metadata(&metadata);
    Ok((metadata, identity))
}

/// Removes a stale same-user Unix socket without following or deleting any
/// other kind of filesystem entry.
pub(crate) async fn prepare_for_bind(path: &Path) -> io::Result<()> {
    let (_, identity) = match inspect_owned_socket(path) {
        Ok(socket) => socket,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };

    match UnixStream::connect(path).await {
        Ok(stream) => {
            drop(stream);
            return Err(error(
                io::ErrorKind::AddrInUse,
                path,
                "GSB socket is already accepting connections",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {}
        Err(connect_error) => {
            return Err(io::Error::new(
                connect_error.kind(),
                format!(
                    "cannot verify whether GSB socket {} is stale: {}",
                    path.display(),
                    connect_error
                ),
            ));
        }
    }

    let (metadata, _) = inspect_owned_socket(path)?;
    if !identity.matches(&metadata) {
        return Err(error(
            io::ErrorKind::Other,
            path,
            "GSB socket changed while checking whether it was stale",
        ));
    }

    fs::remove_file(path).map_err(|remove_error| {
        io::Error::new(
            remove_error.kind(),
            format!(
                "cannot remove stale GSB socket {}: {}",
                path.display(),
                remove_error
            ),
        )
    })
}

/// Applies and verifies owner-only permissions on a newly bound socket.
pub(crate) fn set_owner_only_permissions(path: &Path) -> io::Result<()> {
    let (_, identity) = inspect_owned_socket(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|permission_error| {
        io::Error::new(
            permission_error.kind(),
            format!(
                "cannot set mode 0600 on GSB socket {}: {}",
                path.display(),
                permission_error
            ),
        )
    })?;

    let (metadata, _) = inspect_owned_socket(path)?;
    if !identity.matches(&metadata) || metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(error(
            io::ErrorKind::Other,
            path,
            "GSB socket changed while applying owner-only permissions",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[tokio::test]
    async fn removes_stale_same_user_socket() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("gsb.sock");
        let listener = UnixListener::bind(&path).unwrap();
        drop(listener);

        prepare_for_bind(&path).await.unwrap();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn rejects_live_socket_without_removing_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("gsb.sock");
        let _listener = UnixListener::bind(&path).unwrap();

        let error = prepare_for_bind(&path).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        assert!(path.exists());
    }

    #[tokio::test]
    async fn rejects_regular_file_without_removing_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("gsb.sock");
        fs::write(&path, b"not a socket").unwrap();

        let error = prepare_for_bind(&path).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&path).unwrap(), b"not a socket");
    }

    #[tokio::test]
    async fn rejects_symlink_without_removing_it_or_its_target() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        let path = directory.path().join("gsb.sock");
        fs::write(&target, b"keep me").unwrap();
        symlink(&target, &path).unwrap();

        let error = prepare_for_bind(&path).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&target).unwrap(), b"keep me");
    }

    #[test]
    fn applies_owner_only_socket_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("gsb.sock");
        let _listener = UnixListener::bind(&path).unwrap();

        set_owner_only_permissions(&path).unwrap();
        let metadata = fs::symlink_metadata(&path).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    }
}
