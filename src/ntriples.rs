//! N-Triples: the plainest RDF syntax, one triple per line, each ended by a dot.
//!
//! Everywhere in this project a term is kept exactly as N-Triples writes it:
//!
//! - `<https://example.org/x>`: an IRI
//! - `_:w7`: a blank node
//! - `"brake pressure"@en`: a literal with a language
//! - `"0.5"^^<http://www.w3.org/2001/XMLSchema#double>`: a literal with a datatype
//!
//! This module reads and writes the part of the syntax the graph needs. It does not undo `\u`
//! escapes, and it expects a space between the object and the final dot.

/// Splits one line into subject, predicate and object, each still in N-Triples syntax.
/// Returns `None` for an empty line, a comment or a line that is not a triple.
pub fn parse_line(line: &str) -> Option<[&str; 3]> {
    let rest = line.trim();
    if rest.is_empty() || rest.starts_with('#') {
        return None;
    }
    let (subject, rest) = take_term(rest)?;
    let (predicate, rest) = take_term(rest)?;
    let (object, rest) = take_term(rest)?;
    let well_formed = rest.trim() == "." && !subject.starts_with('"') && predicate.starts_with('<');
    well_formed.then_some([subject, predicate, object])
}

/// Cuts the first term off `text`. Returns the term and what follows it.
fn take_term(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();
    let bytes = text.as_bytes();
    let end = match bytes.first()? {
        b'<' => text.find('>')? + 1,
        b'_' => text.find(char::is_whitespace).unwrap_or(text.len()),
        b'"' => {
            // The closing quote is the first one with no backslash in front of it.
            let mut i = 1;
            while *bytes.get(i)? != b'"' {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            // After it there may be a language (@en) or a datatype (^^<...>).
            let tail = &text[i + 1..];
            let extra = if tail.starts_with("^^<") {
                tail.find('>')? + 1
            } else if tail.starts_with('@') {
                tail.find(char::is_whitespace).unwrap_or(tail.len())
            } else {
                0
            };
            i + 1 + extra
        }
        _ => return None,
    };
    Some((&text[..end], &text[end..]))
}

/// Writes an IRI as a term.
pub fn iri(value: &str) -> String {
    format!("<{value}>")
}

/// The IRI inside a term, or `None` if the term is not an IRI.
pub fn iri_value(term: &str) -> Option<&str> {
    term.strip_prefix('<')?.strip_suffix('>')
}

/// Writes a literal as a term. `kind` is what follows the closing quote: nothing for a plain
/// text, `@en` for a language or `^^<...>` for a datatype.
pub fn literal(text: &str, kind: &str) -> String {
    let mut term = String::with_capacity(text.len() + kind.len() + 2);
    term.push('"');
    for c in text.chars() {
        match c {
            '"' => term.push_str("\\\""),
            '\\' => term.push_str("\\\\"),
            '\n' => term.push_str("\\n"),
            '\r' => term.push_str("\\r"),
            other => term.push(other),
        }
    }
    term.push('"');
    term.push_str(kind);
    term
}

/// The two parts of a literal: its text with the escapes undone, and what follows the closing
/// quote. Returns `None` if the term is not a literal.
pub fn literal_parts(term: &str) -> Option<(String, &str)> {
    let body = term.strip_prefix('"')?;
    let mut text = String::new();
    let mut chars = body.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((text, &body[i + 1..])),
            '\\' => match chars.next()?.1 {
                'n' => text.push('\n'),
                'r' => text.push('\r'),
                't' => text.push('\t'),
                other => text.push(other),
            },
            other => text.push(other),
        }
    }
    None
}

/// The text of a literal, or the term itself if it is not one. For messages and prompts.
pub fn text_of(term: &str) -> String {
    literal_parts(term).map_or_else(|| term.to_string(), |(text, _)| text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_the_three_kinds_of_object() {
        let line = "<https://e.org/s> <https://e.org/p> <https://e.org/o> .";
        assert_eq!(parse_line(line), Some(["<https://e.org/s>", "<https://e.org/p>", "<https://e.org/o>"]));
        let line = "_:w7 <https://e.org/p> \"0.5\"^^<http://www.w3.org/2001/XMLSchema#double> .";
        assert_eq!(
            parse_line(line),
            Some(["_:w7", "<https://e.org/p>", "\"0.5\"^^<http://www.w3.org/2001/XMLSchema#double>"])
        );
        let line = "<https://e.org/s> <https://e.org/p> \"says \\\"no\\\" . twice\"@en .";
        assert_eq!(parse_line(line).unwrap()[2], "\"says \\\"no\\\" . twice\"@en");
    }

    #[test]
    fn rejects_what_is_not_a_triple() {
        assert_eq!(parse_line(""), None);
        assert_eq!(parse_line("# a comment"), None);
        assert_eq!(parse_line("<https://e.org/s> <https://e.org/p> ."), None);
        assert_eq!(parse_line("<https://e.org/s> <https://e.org/p> \"never closed ."), None);
        assert_eq!(parse_line("\"text\" <https://e.org/p> <https://e.org/o> ."), None);
        assert_eq!(parse_line("<https://e.org/s> _:b <https://e.org/o> ."), None);
    }

    #[test]
    fn a_literal_comes_back_as_it_was_written() {
        let term = literal("line one\nsays \"no\" \\ ok", "@en");
        assert_eq!(term, "\"line one\\nsays \\\"no\\\" \\\\ ok\"@en");
        assert_eq!(literal_parts(&term), Some(("line one\nsays \"no\" \\ ok".to_string(), "@en")));
        assert_eq!(literal_parts("<https://e.org/s>"), None);
        assert_eq!(text_of("\"ESP12\""), "ESP12");
        assert_eq!(iri_value(&iri("https://e.org/s")), Some("https://e.org/s"));
    }
}
