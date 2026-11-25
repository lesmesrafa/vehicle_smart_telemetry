//! The live part of the knowledge graph: one slot per signal node, written every time a value
//! arrives. Nothing is allocated and no triple is created on arrival.
//!
//! Each slot keeps a short history as a ring of buckets, one bucket per `bucket_s` seconds of
//! stream time. A bucket holds a summary of what arrived in it, and summaries can be merged, so
//! a window of any whole number of buckets is answered without keeping the samples.

use crate::graph::Graph;
use crate::ntriples::text_of;
use crate::vocab;
use crate::wire::Sample;
use std::collections::HashMap;

/// What one signal did during one bucket of time, or during several merged ones.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    /// How many values arrived.
    pub count: u32,
    pub first: f64,
    pub last: f64,
    /// Stream time of the last value.
    pub last_t: f64,
    pub min: f64,
    pub max: f64,
    pub sum: f64,
}

impl Summary {
    fn start(t: f64, value: f64) -> Summary {
        Summary { count: 1, first: value, last: value, last_t: t, min: value, max: value, sum: value }
    }

    /// Adds a value that arrived after all the ones already counted.
    fn add(&mut self, t: f64, value: f64) {
        self.count += 1;
        self.last = value;
        self.last_t = t;
        self.min = self.min.min(value);
        self.max = self.max.max(value);
        self.sum += value;
    }

    /// Joins this summary with the one of a later stretch of time.
    fn merge(&mut self, later: &Summary) {
        self.count += later.count;
        self.last = later.last;
        self.last_t = later.last_t;
        self.min = self.min.min(later.min);
        self.max = self.max.max(later.max);
        self.sum += later.sum;
    }

    pub fn mean(&self) -> f64 {
        self.sum / f64::from(self.count)
    }

    /// How much the signal moved from its first value to its last.
    pub fn change(&self) -> f64 {
        self.last - self.first
    }
}

/// One bucket of the ring: which stretch of time it holds, and its summary.
#[derive(Clone, Copy)]
struct Bucket {
    /// Number of the stretch: stream time divided by `bucket_s`, rounded down. -1 means empty.
    number: i64,
    summary: Summary,
}

const EMPTY: Bucket = Bucket {
    number: -1,
    summary: Summary { count: 0, first: 0.0, last: 0.0, last_t: 0.0, min: 0.0, max: 0.0, sum: 0.0 },
};

/// The slots of every signal of the graph.
pub struct Live {
    /// Seconds of stream time each bucket covers.
    pub bucket_s: f64,
    /// How many buckets each slot keeps: the longest window that can be asked for.
    pub ring: usize,
    /// Graph number of the signal of each slot, in increasing order.
    nodes: Vec<u32>,
    /// Where a value goes: message name -> signal name -> slot.
    index: HashMap<String, HashMap<String, usize>>,
    /// `ring` buckets per slot, one slot after another.
    buckets: Vec<Bucket>,
    /// Values written into a slot so far.
    pub written: u64,
    /// Values of signals the graph does not have.
    pub unknown: u64,
}

impl Live {
    /// Gives a slot to every node of the graph that is a signal, and finds the names under which
    /// its values arrive: the name of its message and its own.
    pub fn new(graph: &Graph, bucket_s: f64, ring: usize) -> Result<Live, String> {
        let number = |term: &str| graph.number(term).ok_or(format!("the graph has no {term}"));
        let (a, signal, name, in_message) =
            (number(vocab::TYPE)?, number(vocab::SIGNAL)?, number(vocab::NAME)?, number(vocab::IN_MESSAGE)?);

        let nodes: Vec<u32> = graph.subjects(a, signal).collect();
        let mut index: HashMap<String, HashMap<String, usize>> = HashMap::new();
        for (slot, &node) in nodes.iter().enumerate() {
            let own = graph.object(node, name).ok_or(format!("{} has no name", graph.term(node)))?;
            let message = graph.object(node, in_message).ok_or(format!("{} has no message", graph.term(node)))?;
            let message = graph.object(message, name).ok_or(format!("{} has no name", graph.term(message)))?;
            index.entry(text_of(graph.term(message))).or_default().insert(text_of(graph.term(own)), slot);
        }
        let buckets = vec![EMPTY; nodes.len() * ring];
        Ok(Live { bucket_s, ring, nodes, index, buckets, written: 0, unknown: 0 })
    }

    /// How many signals have a slot.
    pub fn slots(&self) -> usize {
        self.nodes.len()
    }

    /// The slot of a graph node, or `None` if that node is not a signal.
    pub fn slot_of(&self, node: u32) -> Option<usize> {
        self.nodes.binary_search(&node).ok()
    }

    /// The graph node of a slot.
    pub fn node(&self, slot: usize) -> u32 {
        self.nodes[slot]
    }

    /// The number of the bucket a stream time falls in.
    pub fn bucket_of(&self, t: f64) -> i64 {
        (t / self.bucket_s).floor() as i64
    }

    /// Takes one decoded CAN message: every value goes to the slot of its signal.
    pub fn accept(&mut self, sample: &Sample) {
        let Some(signals) = self.index.get(sample.message.as_str()) else {
            self.unknown += sample.signals.len() as u64;
            return;
        };
        let number = (sample.t / self.bucket_s).floor() as i64;
        let place = number.rem_euclid(self.ring as i64) as usize;
        for (name, value) in &sample.signals {
            let Some(&slot) = signals.get(name.as_str()) else {
                self.unknown += 1;
                continue;
            };
            let bucket = &mut self.buckets[slot * self.ring + place];
            if bucket.number == number {
                bucket.summary.add(sample.t, *value);
            } else {
                // The bucket still holds an older stretch of time: it starts again.
                *bucket = Bucket { number, summary: Summary::start(sample.t, *value) };
            }
            self.written += 1;
        }
    }

    /// What a signal did in the `count` buckets that end just before bucket number `end`.
    /// Returns `None` if nothing arrived in them.
    pub fn summary(&self, slot: usize, end: i64, count: usize) -> Option<Summary> {
        let mut total: Option<Summary> = None;
        for number in end - count.min(self.ring) as i64..end {
            let bucket = &self.buckets[slot * self.ring + number.rem_euclid(self.ring as i64) as usize];
            if bucket.number != number {
                continue; // nothing arrived in that stretch, or the ring has already reused it
            }
            match total.as_mut() {
                Some(summary) => summary.merge(&bucket.summary),
                None => total = Some(bucket.summary),
            }
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAPH: &str = "\
<https://e.org/s1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://example.org/jedge/voc#Signal> .
<https://e.org/s1> <https://example.org/jedge/voc#name> \"SPEED\" .
<https://e.org/s1> <https://example.org/jedge/voc#inMessage> <https://e.org/m> .
<https://e.org/s2> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://example.org/jedge/voc#Signal> .
<https://e.org/s2> <https://example.org/jedge/voc#name> \"ACCEL\" .
<https://e.org/s2> <https://example.org/jedge/voc#inMessage> <https://e.org/m> .
<https://e.org/m> <https://example.org/jedge/voc#name> \"MOTION\" .
";

    fn sample(t: f64, speed: f64) -> Sample {
        let signals = vec![("SPEED".to_string(), speed), ("OTHER".to_string(), 1.0)];
        Sample { t, id: 1, message: "MOTION".to_string(), signals }
    }

    #[test]
    fn every_signal_of_the_graph_gets_a_slot() {
        let graph = Graph::from_text(GRAPH);
        let live = Live::new(&graph, 1.0, 4).unwrap();
        assert_eq!(live.slots(), 2);
        let speed = graph.number("<https://e.org/s1>").unwrap();
        assert_eq!(live.node(live.slot_of(speed).unwrap()), speed);
        assert_eq!(live.slot_of(graph.number("<https://e.org/m>").unwrap()), None);
    }

    #[test]
    fn a_window_is_the_merge_of_its_buckets() {
        let graph = Graph::from_text(GRAPH);
        let mut live = Live::new(&graph, 1.0, 4).unwrap();
        let slot = live.slot_of(graph.number("<https://e.org/s1>").unwrap()).unwrap();
        for (t, speed) in [(0.2, 10.0), (0.7, 12.0), (1.5, 20.0), (3.1, 8.0), (3.9, 9.0), (4.0, 99.0)] {
            live.accept(&sample(t, speed));
        }
        assert_eq!((live.written, live.unknown), (6, 6));

        // Seconds 0 to 4: buckets 0, 1 and 3 have values, bucket 2 has none, and bucket 4 has
        // reused the place of bucket 0 in a ring of four.
        let window = live.summary(slot, 4, 3).unwrap();
        assert_eq!((window.count, window.first, window.last, window.last_t), (3, 20.0, 9.0, 3.9));
        assert_eq!((window.min, window.max, window.mean(), window.change()), (8.0, 20.0, 37.0 / 3.0, -11.0));
        assert_eq!(live.summary(slot, 4, 4).unwrap().count, 3);
        assert_eq!(live.summary(slot, 3, 1), None);
        assert_eq!(live.summary(slot, 5, 1).unwrap().last, 99.0);

        let mut other = sample(5.0, 1.0);
        other.message = "UNKNOWN".to_string();
        live.accept(&other);
        assert_eq!(live.unknown, 8);
    }
}
