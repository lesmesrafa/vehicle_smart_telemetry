//! The fixed part of the knowledge graph, held in memory the way RDF engines for small devices
//! such as RDF4Led hold it: every term becomes a number and every triple three numbers, kept in
//! three sorted copies so that any pattern is answered with a binary search.
//!
//! It is loaded once from an N-Triples file and never changes while the gateway runs.

use crate::ntriples::parse_line;
use crate::with_name;
use std::fs;
use std::io;

/// A triple as three term numbers. Their order depends on the index the row is in.
pub type Row = [u32; 3];

/// The graph: the dictionary of terms and the three indexes over the triples.
pub struct Graph {
    /// Every term, in N-Triples syntax and sorted. The number of a term is its position here.
    terms: Vec<String>,
    /// The triples as subject, predicate, object.
    spo: Vec<Row>,
    /// The triples as predicate, object, subject.
    pos: Vec<Row>,
    /// The triples as object, subject, predicate.
    osp: Vec<Row>,
}

impl Graph {
    /// Reads an N-Triples file from disk.
    pub fn load(path: &str) -> io::Result<Graph> {
        let text = fs::read_to_string(path).map_err(|error| with_name(path, error))?;
        Ok(Graph::from_text(&text))
    }

    /// Builds the graph from N-Triples text. Lines that are not triples are skipped.
    pub fn from_text(text: &str) -> Graph {
        let triples: Vec<[&str; 3]> = text.lines().filter_map(parse_line).collect();

        // The dictionary: every different term, sorted.
        let mut terms: Vec<&str> = triples.iter().flatten().copied().collect();
        terms.sort_unstable();
        terms.dedup();
        // Every term of a triple is in the dictionary, so the search cannot fail.
        let number = |term: &str| terms.binary_search(&term).unwrap_or(0) as u32;

        let mut spo: Vec<Row> = triples.iter().map(|[s, p, o]| [number(s), number(p), number(o)]).collect();
        spo.sort_unstable();
        spo.dedup();
        let mut pos: Vec<Row> = spo.iter().map(|&[s, p, o]| [p, o, s]).collect();
        pos.sort_unstable();
        let mut osp: Vec<Row> = spo.iter().map(|&[s, p, o]| [o, s, p]).collect();
        osp.sort_unstable();

        Graph { terms: terms.into_iter().map(String::from).collect(), spo, pos, osp }
    }

    /// The number of a term, or `None` if the graph does not have it.
    pub fn number(&self, term: &str) -> Option<u32> {
        self.terms.binary_search_by(|known| known.as_str().cmp(term)).ok().map(|i| i as u32)
    }

    /// The term with that number, in N-Triples syntax.
    pub fn term(&self, number: u32) -> &str {
        &self.terms[number as usize]
    }

    /// How many triples the graph has.
    pub fn triples(&self) -> usize {
        self.spo.len()
    }

    /// How many different terms the graph has.
    pub fn terms(&self) -> usize {
        self.terms.len()
    }

    /// The objects of the triples with that subject and predicate.
    pub fn objects(&self, subject: u32, predicate: u32) -> impl Iterator<Item = u32> + '_ {
        range(&self.spo, &[subject, predicate]).iter().map(|row| row[2])
    }

    /// The first of those objects, for when a thing has only one.
    pub fn object(&self, subject: u32, predicate: u32) -> Option<u32> {
        self.objects(subject, predicate).next()
    }

    /// The subjects of the triples with that predicate and object.
    pub fn subjects(&self, predicate: u32, object: u32) -> impl Iterator<Item = u32> + '_ {
        range(&self.pos, &[predicate, object]).iter().map(|row| row[2])
    }

    /// Every triple that matches a pattern, as subject, predicate, object. `None` matches anything.
    pub fn find(&self, s: Option<u32>, p: Option<u32>, o: Option<u32>) -> Vec<Row> {
        let from_pos = |rows: &[Row]| rows.iter().map(|&[p, o, s]| [s, p, o]).collect();
        let from_osp = |rows: &[Row]| rows.iter().map(|&[o, s, p]| [s, p, o]).collect();
        match (s, p, o) {
            (Some(s), Some(p), Some(o)) => range(&self.spo, &[s, p, o]).to_vec(),
            (Some(s), Some(p), None) => range(&self.spo, &[s, p]).to_vec(),
            (Some(s), None, None) => range(&self.spo, &[s]).to_vec(),
            (None, Some(p), Some(o)) => from_pos(range(&self.pos, &[p, o])),
            (None, Some(p), None) => from_pos(range(&self.pos, &[p])),
            (Some(s), None, Some(o)) => from_osp(range(&self.osp, &[o, s])),
            (None, None, Some(o)) => from_osp(range(&self.osp, &[o])),
            (None, None, None) => self.spo.clone(),
        }
    }
}

/// The rows of a sorted index that begin with `start`.
fn range<'a>(index: &'a [Row], start: &[u32]) -> &'a [Row] {
    let n = start.len();
    let first = index.partition_point(|row| &row[..n] < start);
    let after = index.partition_point(|row| &row[..n] <= start);
    &index[first..after]
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = "\
# three signals, two of them in the same message
<https://e.org/a> <https://e.org/in> <https://e.org/m1> .
<https://e.org/b> <https://e.org/in> <https://e.org/m1> .
<https://e.org/c> <https://e.org/in> <https://e.org/m2> .
<https://e.org/a> <https://e.org/unit> \"km/h\" .
<https://e.org/a> <https://e.org/unit> \"km/h\" .
this line is not a triple
";

    #[test]
    fn loads_and_numbers_the_terms() {
        let graph = Graph::from_text(EXAMPLE);
        assert_eq!(graph.triples(), 4); // the repeated triple counts once
        assert_eq!(graph.terms(), 8);
        let a = graph.number("<https://e.org/a>").unwrap();
        assert_eq!(graph.term(a), "<https://e.org/a>");
        assert_eq!(graph.number("<https://e.org/nothing>"), None);
    }

    #[test]
    fn answers_every_pattern() {
        let graph = Graph::from_text(EXAMPLE);
        let n = |term: &str| graph.number(term).unwrap();
        let (a, b, c) = (n("<https://e.org/a>"), n("<https://e.org/b>"), n("<https://e.org/c>"));
        let (within, unit) = (n("<https://e.org/in>"), n("<https://e.org/unit>"));
        let (m1, kmh) = (n("<https://e.org/m1>"), n("\"km/h\""));

        assert_eq!(graph.object(a, unit), Some(kmh));
        assert_eq!(graph.object(b, unit), None);
        assert_eq!(graph.subjects(within, m1).collect::<Vec<u32>>(), [a, b]);

        assert_eq!(graph.find(Some(a), Some(within), Some(m1)), [[a, within, m1]]);
        assert_eq!(graph.find(Some(a), Some(unit), None), [[a, unit, kmh]]);
        assert_eq!(graph.find(Some(a), None, None).len(), 2);
        assert_eq!(graph.find(None, Some(within), Some(m1)), [[a, within, m1], [b, within, m1]]);
        assert_eq!(graph.find(None, Some(within), None).len(), 3);
        assert_eq!(graph.find(Some(c), None, Some(m1)), Vec::<Row>::new());
        assert_eq!(graph.find(Some(b), None, Some(m1)), [[b, within, m1]]);
        assert_eq!(graph.find(None, None, Some(kmh)), [[a, unit, kmh]]);
        assert_eq!(graph.find(None, None, None).len(), 4);
    }
}
