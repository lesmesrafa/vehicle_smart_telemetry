//! The wire format: one decoded CAN message per line, written as JSON.
//!
//! `{"t":12.345678,"id":544,"msg":"ESP12","sig":{"LAT_ACCEL":0.03,"LONG_ACCEL":-0.13}}`
//!
//! - `t`: seconds since the first frame of the recording
//! - `id`: CAN identifier, in decimal
//! - `msg`: name of the message in the DBC
//! - `sig`: every signal that frame carries, already as a physical value
//!
//! Any JSON library reads these lines. `decode` below is not a general JSON parser: it reads
//! exactly what `encode` writes, which is all the two ends of this testbed need.

use std::fmt::Write;
/// One decoded CAN message: when it was sent, which message it is and the value of each signal.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub t: f64,
    pub id: u32,
    pub message: String,
    pub signals: Vec<(String, f64)>,
}

/// Whether a name can go on the wire as it is: only letters, digits and underscores.
pub fn is_plain(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Appends one line to `line`, without the final newline. Returns how many signals it wrote.
/// The names must be plain (see `is_plain`). A value that is not a finite number is left out.
pub fn encode<'a>(
    line: &mut String,
    t: f64,
    id: u32,
    message: &str,
    signals: impl IntoIterator<Item = (&'a str, f64)>,
) -> usize {
    // Writing into a String cannot fail, so the results of `write!` are ignored.
    let _ = write!(line, "{{\"t\":{t:.6},\"id\":{id},\"msg\":\"{message}\",\"sig\":{{");
    let mut written = 0;
    for (name, value) in signals {
        if !value.is_finite() {
            continue;
        }
        if written > 0 {
            line.push(',');
        }
        let _ = write!(line, "\"{name}\":{value}");
        written += 1;
    }
    line.push_str("}}");
    written
}

/// Reads back a line written by `encode`. Returns `None` if the line does not have that form.
pub fn decode(line: &str) -> Option<Sample> {
    let rest = line.trim_end().strip_prefix("{\"t\":")?;
    let (t, rest) = rest.split_once(",\"id\":")?;
    let (id, rest) = rest.split_once(",\"msg\":\"")?;
    let (message, rest) = rest.split_once("\",\"sig\":{")?;
    let body = rest.strip_suffix("}}")?;
    let mut signals = Vec::new();
    if !body.is_empty() {
        for pair in body.split(',') {
            let (name, value) = pair.split_once(':')?;
            let name = name.strip_prefix('"')?.strip_suffix('"')?;
            signals.push((name.to_string(), value.parse().ok()?));
        }
    }
    Some(Sample { t: t.parse().ok()?, id: id.parse().ok()?, message: message.to_string(), signals })
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_documented_line() {
        let mut line = String::new();
        let n = encode(&mut line, 12.345678, 544, "ESP12", [("LAT_ACCEL", 0.03), ("LONG_ACCEL", -0.13)]);
        assert_eq!(n, 2);
        assert_eq!(line, r#"{"t":12.345678,"id":544,"msg":"ESP12","sig":{"LAT_ACCEL":0.03,"LONG_ACCEL":-0.13}}"#);
    }

    #[test]
    fn what_is_written_is_read_back_exactly() {
        let values = [0.02999999999999936, -10.23, 0.0, 4660.0, 1e-7, 65535.0 * 0.03125];
        let signals: Vec<(String, f64)> = values.iter().enumerate().map(|(i, &v)| (format!("S_{i}"), v)).collect();
        let sample = Sample { t: 1.5, id: 902, message: "WHL_SPD11".to_string(), signals };
        let mut line = String::new();
        encode(&mut line, sample.t, sample.id, &sample.message, sample.signals.iter().map(|(n, v)| (n.as_str(), *v)));
        assert_eq!(decode(&line), Some(sample.clone()));
        line.push('\n');
        assert_eq!(decode(&line), Some(sample));
    }

    #[test]
    fn a_message_without_signals_and_a_value_that_is_not_a_number() {
        let mut line = String::new();
        assert_eq!(encode(&mut line, 0.0, 1, "EMPTY", [("BAD", f64::NAN)]), 0);
        assert_eq!(line, r#"{"t":0.000000,"id":1,"msg":"EMPTY","sig":{}}"#);
        assert_eq!(decode(&line).unwrap().signals, []);
    }

    #[test]
    fn rejects_what_is_not_a_line_of_ours() {
        assert!(decode("").is_none());
        assert!(decode("this is not a sample").is_none());
        assert!(decode(r#"{"t":1.0,"id":544,"msg":"ESP12","sig":{"LAT_ACCEL":0.03"#).is_none());
        assert!(decode(r#"{"t":1.0,"id":544,"msg":"ESP12","sig":{"LAT_ACCEL":abc}}"#).is_none());
        assert!(is_plain("WHL_SPD_FL") && !is_plain("km/h") && !is_plain(""));
    }
}