//! Command-line option parsing, without external libraries.

use std::collections::HashMap;
use std::process;

/// The parsed options: those that take a value, the switches and the positional arguments.
pub struct Args {
    values: HashMap<String, String>,
    switches: Vec<String>,
    pub positional: Vec<String>,
}
impl Args {
    /// Parses the program arguments. `with_value` and `without_value` are the accepted options.
    /// With `--help`, or on an unknown option, it prints a message and ends the program.
    pub fn parse(help: &str, with_value: &[&str], without_value: &[&str]) -> Args {
        let mut args = Args { values: HashMap::new(), switches: Vec::new(), positional: Vec::new() };
        let mut input = std::env::args().skip(1);
        while let Some(word) = input.next() {
            if word == "--help" || word == "-h" {
                println!("{help}");
                process::exit(0);
            } else if with_value.contains(&word.as_str()) {
                match input.next() {
                    Some(value) => {
                        args.values.insert(word, value);
                    }
                    None => fail(&format!("option {word} is missing its value")),
                }
            } else if without_value.contains(&word.as_str()) {
                args.switches.push(word);
            } else if word.starts_with("--") {
                fail(&format!("unknown option {word}; try --help"));
            } else {
                args.positional.push(word);
            }
        }
        args
    }

    /// The value of an option, or `default` if it was not given.
    pub fn text(&self, name: &str, default: &str) -> String {
        self.values.get(name).cloned().unwrap_or_else(|| default.to_string())
    }

    /// The value of an option, only if it was given.
    pub fn optional(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    /// The value of an option as a number, or `default` if it was not given.
    pub fn number(&self, name: &str, default: f64) -> f64 {
        match self.values.get(name) {
            None => default,
            Some(text) => {
                text.parse().unwrap_or_else(|_| fail(&format!("{name} expects a number and got '{text}'")))
            }
        }
    }

    /// Whether a switch was given.
    pub fn enabled(&self, name: &str) -> bool {
        self.switches.iter().any(|s| s == name)
    }
}
/// Prints an error message and ends the program.
pub fn fail(message: &str) -> ! {
    eprintln!("error: {message}");
    process::exit(2)
}
