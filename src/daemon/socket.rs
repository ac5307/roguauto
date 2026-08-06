// Copyright (C) 2026 ac5307
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::{env, io, Path};

const SOCKET_FILE_NAME: &str = "control.sock";

pub fn socket_path() -> io::Result<Box<Path>> {
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
