// SPDX-License-Identifier: Apache-2.0 OR MIT
// Library of the module testbed: what the gateway, ECUs and event counter share.
pub mod candump;
pub mod dbc;
pub mod metrics;
pub mod options;
pub mod wire;
pub mod timing;
pub mod ntriples;
pub mod vocab;
pub mod defaults;
pub mod graph;
pub mod live;
pub mod question;
pub mod jelly;
pub mod model;

use std::io;

/// Port the gateway listens on when no other is given.
pub const DEFAULT_PORT: u16 = 7700;

/// Adds to an I/O error the name of the file or bus that caused it
pub fn with_name(name: &str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{}: {}", name, error))
}