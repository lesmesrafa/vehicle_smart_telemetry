//! Reading a DBC file and decoding CAN frames using it.

use std::collections::HashMap;
use std::fs;
use std::io;

/// Role of a signal in a multiplexed message
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mux {
    /// Plain signal, present in every frame of the message
    No,
    /// Selector signal, its value says which group of signals this frame carries
    Selector,
    /// Signal that is present only when the selector holds this number
    Case(u64)
}

/// A signal: where its bits are and how they become a physical quantity.
#[derive(Debug, Clone)]
pub struct Signal {
    pub name: String,
    pub start: u32,
    pub bits: u32,
    pub intel: bool,
    pub signed: bool,
    pub factor: f64,
    pub offset: f64,
    pub unit: String,
    pub receivers: Vec<String>,
    pub mux: Mux,
}

/// A message: its identifier, its name, who sends it, and which signals it carries.
#[derive(Debug, Clone)]
pub struct Message {
    pub id: u32,
    pub name: String,
    pub sender: String,
    pub signals: Vec<Signal>,
}

/// The content of a DBC file: all the messages it defines, indexed by their identifier
#[derive(Debug, Default)]
pub struct Dbc {
    pub messages: Vec<Message>,
    index: HashMap<u32, usize>
}

impl Signal {
    /// Extracts the signal's integer from the frame not yet scaled to a physical quantity.
    pub fn raw(&self, data: &[u8; 8]) -> u64 {
        let mask = if self.bits == 64 { u64::MAX } else { (1u64 << self.bits) - 1 };
        if self.intel {
            (u64::from_le_bytes(*data) >> self.start) & mask
        } else {
            let mut value = 0u64;
            let mut position = self.start;
            for _ in 0..self.bits {
                let (byte, bit) = ((position / 8) as usize, position % 8);
                let octet = data.get(byte).copied().unwrap_or(0);
                value = (value << 1) | u64::from((octet >> bit) & 1);
                position = if bit == 0 { position + 15 } else { position - 1 };
            }
            value
        }
    }

    /// Turns the raw number into the physical quantity, using the sign, factor and offset
    pub fn scale(&self, raw: u64) -> f64 {
        let integer = if self.signed {
            let gap = 64 - self.bits;
            ((raw << gap) as i64 >> gap) as f64
        } else {
            raw as f64
        };
        integer * self.factor + self.offset
    }
}

impl Message {
    /// Value of signal number `i`, or `None` if it is multiplexed and this frame does not carry it.
    pub fn value(&self, i: usize, data: &[u8; 8]) -> Option<f64> {
        let signal = &self.signals[i];
        if let Mux::Case(case) = signal.mux {
            let selector = self.signals.iter().find(|s| s.mux == Mux::Selector)?;
            if selector.raw(data) != case {
                return None;
            }
        }
        Some(signal.scale(signal.raw(data)))
    }

    /// Position in `signals` of the signal with that name, if there is one.
    pub fn position(&self, name: &str) -> Option<usize> {
        self.signals.iter().position(|s| s.name == name)
    }
}

impl Dbc {
    /// Reads a DBC file from disk.
    pub fn load(path: &str) -> io::Result<Dbc> {
        let bytes = fs::read(path).map_err(|error| crate::with_name(path, error))?;
        Ok(Dbc::from_text(&String::from_utf8_lossy(&bytes)))
    }

    /// Parses the text of a DBC. It only looks at `BO_` (message) and `SG_` (signal) lines.
    pub fn from_text(text: &str) -> Dbc {
        let mut dbc = Dbc::default();
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("BO_ ") {
                if let Some(message) = parse_message(rest) {
                    dbc.index.insert(message.id, dbc.messages.len());
                    dbc.messages.push(message);
                }
            } else if let Some(rest) = line.strip_prefix("SG_ ") {
                if let (Some(signal), Some(message)) = (parse_signal(rest), dbc.messages.last_mut()) {
                    message.signals.push(signal);
                }
            }
        }
        dbc
    }

    /// The message with that identifier, if the DBC has it.
    pub fn by_id(&self, id: u32) -> Option<&Message> {
        self.index.get(&id).map(|&i| &self.messages[i])
    }

    /// The message with that name, if the DBC has it.
    pub fn by_name(&self, name: &str) -> Option<&Message> {
        self.messages.iter().find(|m| m.name == name)
    }
}


/// From `544 ESP12: 8 ESC` builds a message, with no signals yet.
fn parse_message(rest: &str) -> Option<Message> {
    let (header, tail) = rest.split_once(':')?;
    let mut words = header.split_whitespace();
    let id: u32 = words.next()?.parse().ok()?;
    let name = words.next()?.to_string();
    let sender = tail.split_whitespace().nth(1).unwrap_or("").to_string();
    Some(Message { id: id & 0x1FFF_FFFF, name, sender, signals: Vec::new() })
}


/// From `LONG_ACCEL : 13|11@1+ (0.01,-10.23) [-10.23|10.24] "m/s^2"  EMS,TCU` builds a signal.
fn parse_signal(rest: &str) -> Option<Signal> {
    let (header, definition) = rest.split_once(':')?;
    let mut words = header.split_whitespace();
    let name = words.next()?.to_string();
    let mux = match words.next() {
        None => Mux::No,
        Some("M") => Mux::Selector,
        Some(mark) => Mux::Case(mark.strip_prefix('m')?.parse().ok()?),
    };
    let mut pieces = definition.split('"');
    let mut numbers = pieces.next()?.split_whitespace();
    let unit = pieces.next()?.to_string();
    let receivers = pieces
        .next()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|r| !r.is_empty() && *r != "Vector__XXX")
        .map(String::from)
        .collect();
    let (start, tail) = numbers.next()?.split_once('|')?;
    let (bits, order) = tail.split_once('@')?;
    let (start, bits): (u32, u32) = (start.parse().ok()?, bits.parse().ok()?);
    let scale = numbers.next()?.trim_matches(|c| c == '(' || c == ')');
    let (factor, offset) = scale.split_once(',')?;
    let intel = order.starts_with('1');
    if bits == 0 || bits > 64 || start > 63 || (intel && start + bits > 64) {
        return None;
    }
    Some(Signal {
        name,
        start,
        bits,
        intel,
        signed: order.ends_with('-'),
        factor: factor.parse().ok()?,
        offset: offset.parse().ok()?,
        unit,
        receivers,
        mux,
    })
}



#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
    BO_ 544 ESP12: 8 ESC
    SG_ LAT_ACCEL : 0|11@1+ (0.01,-10.23) [-10.23|10.24] "m/s^2"  EMS,TCU
    SG_ LONG_ACCEL : 13|11@1+ (0.01,-10.23) [-10.23|10.24] "m/s^2"  EMS
    BO_ 100 TEST: 8 XXX
    SG_ SIGNED : 0|12@1- (0.5,0) [0|0] ""  A
    SG_ MOTOROLA : 23|16@0+ (1,0) [0|0] "km/h or MPH"  A
    SG_ SELECTOR M : 32|4@1+ (1,0) [0|0] ""  A
    SG_ CASE_TWO m2 : 36|4@1+ (1,0) [0|0] ""  A
    SG_ CASE_THREE m3 : 36|4@1+ (1,0) [0|0] ""  A
    "#;

    #[test]
    fn decodes_like_cantools() {
        let dbc = Dbc::from_text(EXAMPLE);
        let esp12 = dbc.by_id(544).unwrap();
        let data = [0x02, 0x84, 0x81, 0x00, 0x00, 0xFF, 0x0F, 0x00];
        assert_eq!(esp12.value(0, &data), Some(0.02999999999999936));
        assert_eq!(esp12.value(1, &data), Some(0.129999999999999));
        assert_eq!(esp12.signals[0].receivers, ["EMS", "TCU"]);

        let test = dbc.by_name("TEST").unwrap();
        let data = [0xFE, 0x0F, 0x12, 0x34, 0x52, 0x00, 0x00, 0x00];
        let values: Vec<Option<f64>> = (0..5).map(|i| test.value(i, &data)).collect();
        assert_eq!(values, [Some(-1.0), Some(4660.0), Some(2.0), Some(5.0), None]);
        assert_eq!(test.signals[1].unit, "km/h or MPH");
    }
}