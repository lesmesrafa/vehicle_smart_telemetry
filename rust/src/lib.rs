// SPDX-License-Identifier: Apache-2.0 OR MIT
// Library of the module testbed: what the gateway, ECUs and event counter share.
pub mod candump;
pub mod dbc;
pub mod options;

use std::io;

/// Adds to an I/O error the name of the file or bus that caused it
pub fn with_name(name: &str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{}: {}", name, error))
}