//! Jelly: a binary format for streams of RDF, built on Protocol Buffers and made for moving RDF
//! between small devices and servers. This module writes the row layout of Jelly-RDF 1.0, which
//! every Jelly reader accepts, as a stream of graphs: one frame per graph.
//!
//! What goes into the file, from the outside in:
//!
//! - the stream is a sequence of frames, each preceded by its length;
//! - a frame is a list of rows;
//! - a row is one of: the options of the stream (only the first row), an entry of a lookup
//!   table (a prefix, a name or a datatype) or a triple;
//! - a triple names its IRIs by two numbers, the prefix and the name, which point into the
//!   lookup tables. A long IRI is written once and costs two small numbers ever after.
//!
//! The lookup tables here only grow: each new text gets the next number. That is enough for a
//! graph whose vocabulary is fixed, and it means no entry ever has to carry its number.

use crate::ntriples::{iri_value, literal_parts};
use std::collections::HashMap;
use std::io::{self, Write};

// Sizes of the lookup tables promised to the reader in the options of the stream.
const MAX_NAMES: u32 = 4000;
const MAX_PREFIXES: u32 = 150;
const MAX_DATATYPES: u32 = 32;

// Field numbers of the messages, from the rdf.proto file of the Jelly specification.
const FRAME_ROWS: u32 = 1;
const ROW_OPTIONS: u32 = 1;
const ROW_TRIPLE: u32 = 2;
const ROW_NAME: u32 = 9;
const ROW_PREFIX: u32 = 10;
const ROW_DATATYPE: u32 = 11;
const ENTRY_VALUE: u32 = 2;
// In a triple: 1 to 3 are the subject as IRI, blank node or literal; the predicate starts at 5
// and the object at 9. So the field is 4 times the position plus the kind of term.
const KIND_IRI: u32 = 1;
const KIND_BLANK: u32 = 2;
const KIND_LITERAL: u32 = 3;

/// Appends a number in the variable-length form of Protocol Buffers: seven bits per byte, and
/// the top bit says whether another byte follows.
fn varint(buffer: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        buffer.push(value as u8 | 0x80);
        value >>= 7;
    }
    buffer.push(value as u8);
}

/// Appends a field that holds a number: the field number with type 0, then the number.
fn number_field(buffer: &mut Vec<u8>, field: u32, value: u32) {
    varint(buffer, u64::from(field << 3));
    varint(buffer, u64::from(value));
}

/// Appends a field that holds bytes, a text or another message: the field number with type 2,
/// the length and the bytes.
fn bytes_field(buffer: &mut Vec<u8>, field: u32, content: &[u8]) {
    varint(buffer, u64::from(field << 3 | 2));
    varint(buffer, content.len() as u64);
    buffer.extend_from_slice(content);
}

/// Appends a row to a frame: the frame's `rows` field, holding a row with one field set.
fn row(frame: &mut Vec<u8>, field: u32, content: &[u8]) {
    let mut inner = Vec::with_capacity(content.len() + 4);
    bytes_field(&mut inner, field, content);
    bytes_field(frame, FRAME_ROWS, &inner);
}

/// The number of a text in a lookup table. A text that is new gets the next number, and a row
/// that gives it to the reader goes into the frame.
fn lookup(table: &mut HashMap<String, u32>, frame: &mut Vec<u8>, field: u32, text: &str, max: u32) -> io::Result<u32> {
    if let Some(&id) = table.get(text) {
        return Ok(id);
    }
    let id = table.len() as u32 + 1;
    if id > max {
        return Err(io::Error::other(format!("the Jelly lookup table of row {field} is full: {max} entries")));
    }
    table.insert(text.to_string(), id);
    // The entry carries only its text: a reader numbers the entries one after another.
    let mut entry = Vec::with_capacity(text.len() + 4);
    bytes_field(&mut entry, ENTRY_VALUE, text.as_bytes());
    row(frame, field, &entry);
    Ok(id)
}

/// Writes a stream of RDF graphs in Jelly.
pub struct Writer<W: Write> {
    out: W,
    /// The rows of the frame being built.
    frame: Vec<u8>,
    prefixes: HashMap<String, u32>,
    names: HashMap<String, u32>,
    datatypes: HashMap<String, u32>,
    /// The terms of the previous triple of this frame: a term that repeats is not written again.
    previous: [String; 3],
    /// Whether the options of the stream have been written.
    started: bool,
    /// Bytes and frames written so far.
    pub bytes: u64,
    pub frames: u64,
}

impl<W: Write> Writer<W> {
    pub fn new(out: W) -> Writer<W> {
        Writer {
            out,
            frame: Vec::new(),
            prefixes: HashMap::new(),
            names: HashMap::new(),
            datatypes: HashMap::new(),
            previous: Default::default(),
            started: false,
            bytes: 0,
            frames: 0,
        }
    }

    /// Adds a triple to the graph being written. The terms come in N-Triples syntax.
    pub fn triple(&mut self, terms: [&str; 3]) -> io::Result<()> {
        if !self.started {
            self.started = true;
            let mut options = Vec::new();
            number_field(&mut options, 2, 1); // physical type: triples
            number_field(&mut options, 9, MAX_NAMES);
            number_field(&mut options, 10, MAX_PREFIXES);
            number_field(&mut options, 11, MAX_DATATYPES);
            number_field(&mut options, 14, 3); // logical type: a stream of graphs, one per frame
            number_field(&mut options, 15, 1); // version of the protocol: Jelly-RDF 1.0
            row(&mut self.frame, ROW_OPTIONS, &options);
        }
        let mut triple = Vec::new();
        for (position, term) in terms.into_iter().enumerate() {
            if self.previous[position] == term {
                continue; // the reader repeats the term of the previous triple
            }
            let (kind, content) = self.term(term)?;
            bytes_field(&mut triple, 4 * position as u32 + kind, &content);
            self.previous[position] = term.to_string();
        }
        row(&mut self.frame, ROW_TRIPLE, &triple);
        Ok(())
    }

    /// Encodes one term. Returns its kind and its bytes, after adding to the frame the lookup
    /// entries it needs.
    fn term(&mut self, term: &str) -> io::Result<(u32, Vec<u8>)> {
        let mut content = Vec::new();
        if let Some(iri) = iri_value(term) {
            // An IRI is cut after its last # or /: what comes before is the prefix.
            let cut = iri.rfind(['#', '/']).map_or(0, |i| i + 1);
            let prefix = lookup(&mut self.prefixes, &mut self.frame, ROW_PREFIX, &iri[..cut], MAX_PREFIXES)?;
            let name = lookup(&mut self.names, &mut self.frame, ROW_NAME, &iri[cut..], MAX_NAMES)?;
            number_field(&mut content, 1, prefix);
            number_field(&mut content, 2, name);
            Ok((KIND_IRI, content))
        } else if let Some(label) = term.strip_prefix("_:") {
            Ok((KIND_BLANK, label.as_bytes().to_vec()))
        } else if let Some((text, kind)) = literal_parts(term) {
            bytes_field(&mut content, 1, text.as_bytes());
            if let Some(language) = kind.strip_prefix('@') {
                bytes_field(&mut content, 2, language.as_bytes());
            } else if let Some(datatype) = kind.strip_prefix("^^").and_then(iri_value) {
                let id = lookup(&mut self.datatypes, &mut self.frame, ROW_DATATYPE, datatype, MAX_DATATYPES)?;
                number_field(&mut content, 3, id);
            }
            Ok((KIND_LITERAL, content))
        } else {
            Err(io::Error::other(format!("'{term}' is not an RDF term")))
        }
    }

    /// Closes the graph being written: its frame goes out, preceded by its length.
    pub fn end_graph(&mut self) -> io::Result<()> {
        if self.frame.is_empty() {
            return Ok(());
        }
        let mut length = Vec::with_capacity(5);
        varint(&mut length, self.frame.len() as u64);
        self.out.write_all(&length)?;
        self.out.write_all(&self.frame)?;
        self.bytes += (length.len() + self.frame.len()) as u64;
        self.frames += 1;
        self.frame.clear();
        // Each frame can be read with the lookup tables alone, without the triples before it.
        self.previous = Default::default();
        Ok(())
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_take_seven_bits_per_byte() {
        let mut buffer = Vec::new();
        for value in [0, 1, 127, 128, 300, 4000] {
            varint(&mut buffer, value);
        }
        assert_eq!(buffer, [0x00, 0x01, 0x7F, 0x80, 0x01, 0xAC, 0x02, 0xA0, 0x1F]);
    }

    #[test]
    fn writes_the_bytes_of_the_specification() {
        let mut writer = Writer::new(Vec::new());
        writer.triple(["_:w1", "<https://e.org/v#p>", "\"4.5\"^^<https://e.org/x#double>"]).unwrap();
        writer.triple(["_:w1", "<https://e.org/v#p>", "\"hi\"@en"]).unwrap();
        writer.end_graph().unwrap();
        let expected: Vec<u8> = [
            &[0x70][..], // length of the frame: 112 bytes
            // row with the options: triples, 4000 names, 150 prefixes, 32 datatypes, graphs, version 1
            &[0x0A, 0x10, 0x0A, 0x0E, 0x10, 0x01, 0x48, 0xA0, 0x1F, 0x50, 0x96, 0x01, 0x58, 0x20, 0x70, 0x03, 0x78, 0x01],
            // row with prefix 1
            &[0x0A, 0x14, 0x52, 0x12, 0x12, 0x10],
            b"https://e.org/v#",
            // row with name 1
            &[0x0A, 0x05, 0x4A, 0x03, 0x12, 0x01],
            b"p",
            // row with datatype 1
            &[0x0A, 0x1A, 0x5A, 0x18, 0x12, 0x16],
            b"https://e.org/x#double",
            // row with the first triple: blank node w1, IRI (prefix 1, name 1), literal 4.5 of datatype 1
            &[0x0A, 0x15, 0x12, 0x13, 0x12, 0x02],
            b"w1",
            &[0x2A, 0x04, 0x08, 0x01, 0x10, 0x01, 0x5A, 0x07, 0x0A, 0x03],
            b"4.5",
            &[0x18, 0x01],
            // row with the second triple: only the object, a literal in English
            &[0x0A, 0x0C, 0x12, 0x0A, 0x5A, 0x08, 0x0A, 0x02],
            b"hi",
            &[0x12, 0x02],
            b"en",
        ]
        .concat();
        assert_eq!(writer.out, expected);
        assert_eq!((writer.bytes, writer.frames), (expected.len() as u64, 1));

        // The next frame does not repeat the options or the lookup entries, and it starts by
        // writing its three terms again.
        writer.triple(["_:w1", "<https://e.org/v#p>", "\"hi\"@en"]).unwrap();
        writer.end_graph().unwrap();
        let second = &writer.out[expected.len()..];
        assert_eq!(second[..7], [0x18, 0x0A, 0x16, 0x12, 0x14, 0x12, 0x02]);
        assert_eq!(writer.frames, 2);
    }

    #[test]
    fn refuses_what_is_not_a_term() {
        let mut writer = Writer::new(Vec::new());
        assert!(writer.triple(["no term", "<https://e.org/p>", "<https://e.org/o>"]).is_err());
    }
}
