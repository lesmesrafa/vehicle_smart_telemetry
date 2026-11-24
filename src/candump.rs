// CAN frames, and parsing of lines in the format written by "candump -l"
/// A CAN frame: time, identifier, and up to 8 bytes of data. The time is a floating point number in seconds, the identifier is a hexadecimal number, and the data is a sequence of hexadecimal bytes.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub time: f64,
    pub id: u32,
    pub data: [u8; 8],
    pub len: usize,
}

impl Frame {
    /// Builds a frame by copying the bytes it is given (8 at most).
    pub fn new(t: f64, id: u32, bytes: &[u8]) -> Frame {
        let len = bytes.len().min(8);
        let mut data = [0u8; 8];
        data[..len].copy_from_slice(&bytes[..len]);
        Frame { 
            time: t,
            id, 
            data, 
            len 
        }
    }
}

/// Parses a line of the form " (123.456) 123#DEADBEEF" into a Frame.
/// Returns None if the line is not in the expected format.
pub fn parse_line(line: &str) -> Option<Frame> {
    let mut parts = line.split_whitespace();
    let t: f64 = parts.next()?.strip_prefix('(')?.strip_suffix(')')?.parse().ok()?;
    let _bus = parts.next()?;
    let (id_hex, data_hex) = parts.next()?.split_once('#')?;
    let id = u32::from_str_radix(id_hex, 16).ok()?;
    if data_hex.len() % 2 != 0 || data_hex.len() > 16 {
        return None;
    }
    let len = data_hex.len() / 2;
    let mut data = [0u8; 8];
    for (i, slot) in data.iter_mut().take(len).enumerate() {
        *slot = u8::from_str_radix(&data_hex[i*2..i*2+2], 16).ok()?;
    }
    Some(Frame { 
        time: t,
        id,
        data,
        len 
    })
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_line() {
        let frame = parse_line("(1700000000.000700) vcan0 2B0#0000000000").unwrap();
        assert_eq!(frame.id, 0x2B0);
        assert_eq!(frame.len, 5);
        assert_eq!(frame.data, [0; 8]);
    }

    #[test]
    fn broken_line() {
        assert!(parse_line("this is not a frame").is_none());
        assert!(parse_line("(1.0) vcan0 123#ABC").is_none());
    }
}

