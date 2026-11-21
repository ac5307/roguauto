//! Location of the current user's control socket.

use std::{env, io, path::Path};

const SOCKET_FILE_NAME: &str = "control.sock";

/// Resolves the daemon socket within the user's runtime directory.
///
/// Returns an error if `XDG_RUNTIME_DIR` is missing or empty.
pub(crate) fn socket_path() -> io::Result<Box<Path>> {
  use io::{Error, ErrorKind};
  let dir = env::var_os("XDG_RUNTIME_DIR")
    .filter(|val| !val.is_empty())
    .ok_or_else(|| {
      Error::new(ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set")
    })?;

  let path: &Path = dir.as_ref();

  Ok(Box::from(
    path.join(crate::NAMESPACE).join(SOCKET_FILE_NAME),
  ))
}
