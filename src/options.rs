//! Command-line option parsing, without external libraries.

use std::collections::HashMap;
use std::process;

/// The parsed options: those that take a value, the switches and the positional arguments.
pub struct Args {
    values: HashMap<String, String>,
    switches: Vec<String>,
    pub positional: Vec<String>,
}