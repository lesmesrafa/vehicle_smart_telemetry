//! A question registered on the gateway, and the part of the knowledge graph it needs.
//!
//! A question names quantities (vehicle speed, brake pressure). The graph says which signals of
//! this car observe them, what each one is and which equations tie them together. That subgraph
//! is written once as text, the prefix. After that, every window only adds what the signals did:
//! a few short lines, or the same as RDF observations.
//!
//! A question can also name one signal of this car, as MESSAGE.SIGNAL. And where the graph only
//! has a default reading of a signal, one nobody has checked against its values, the prefix
//! says so.

use crate::graph::Graph;
use crate::live::Live;
use crate::ntriples::{iri, literal, text_of};
use crate::vocab;
use std::fmt::Write;

/// One signal of the subgraph of a question.
pub struct Member {
    /// Where its values live.
    pub slot: usize,
    /// Graph number of the quantity it observes, if the graph gives it one.
    pub quantity: Option<u32>,
    /// How many decimals show its values at the resolution of the signal.
    pub decimals: usize,
}

/// A registered question.
pub struct Question {
    pub id: String,
    /// Length of its window, in buckets.
    pub window: usize,
    /// How often a window is produced, in buckets.
    pub slide: usize,
    /// Number of the bucket at which its next window ends.
    pub next_end: i64,
    /// The signals of its subgraph, in the order in which the model reads them.
    pub signals: Vec<Member>,
    /// The fixed text: the signals, their relations, the question and its options.
    pub prefix: String,
}

impl Question {
    /// Registers the question written on one line of the questions file:
    /// identifier, window in seconds, slide in seconds, quantities or signals separated by commas,
    /// the question and its options separated by commas, with a tab between each of the six.
    pub fn register(line: &str, graph: &Graph, live: &Live) -> Result<Question, String> {
        Question::register_as(line, graph, live, false)
    }

    /// The same and, with `bare`, a prefix that leaves out what the graph knows: each signal is
    /// named as the DBC names it and nothing else is said of it, and no relation is given. The
    /// signals and the windows are the same, so the two prefixes can be compared.
    pub fn register_as(line: &str, graph: &Graph, live: &Live, bare: bool) -> Result<Question, String> {
        let fields: Vec<&str> = line.split('\t').map(str::trim).collect();
        let [id, window_s, slide_s, quantities, text, options] = fields[..] else {
            return Err(format!("expected 6 fields separated by tabs and found {}", fields.len()));
        };
        let buckets = |seconds: &str| -> Result<usize, String> {
            let value: f64 = seconds.parse().map_err(|_| format!("'{seconds}' is not a number of seconds"))?;
            let count = (value / live.bucket_s).round();
            if count < 1.0 {
                return Err(format!("{seconds} s is shorter than one bucket ({} s)", live.bucket_s));
            }
            Ok(count as usize)
        };
        let (window, slide) = (buckets(window_s)?, buckets(slide_s)?);
        if window > live.ring {
            return Err(format!("a window of {window_s} s does not fit in the {} buckets kept per signal", live.ring));
        }

        let number = |term: &str| graph.number(term).ok_or(format!("the graph has no {term}"));
        let text_at = |node: u32, property: u32| graph.object(node, property).map(|o| text_of(graph.term(o)));
        let (a, label, unit, name) =
            (number(vocab::TYPE)?, number(vocab::LABEL)?, number(vocab::UNIT)?, number(vocab::NAME)?);
        let (observes, in_message, sent_by, resolution) = (
            number(vocab::OBSERVES)?,
            number(vocab::IN_MESSAGE)?,
            number(vocab::SENT_BY)?,
            number(vocab::RESOLUTION)?,
        );

        // A graph built without the table of default quantities does not have this property.
        let by_default = graph.number(vocab::OBSERVES_BY_DEFAULT);

        // The nodes: the signals the question asks for, by the quantity they observe or by name.
        let mut prefix = String::from("Signals of the vehicle:\n");
        let mut signals: Vec<Member> = Vec::new();
        let mut any_assumed = false;
        for asked in quantities.split(',').map(str::trim) {
            // Each one found: the signal, its quantity and whether that quantity is only assumed.
            let mut found: Vec<(u32, Option<u32>, bool)> = Vec::new();
            if asked.contains('.') {
                // MESSAGE.SIGNAL: that signal of this car, whatever the graph knows of it.
                let node = iri(&format!("{}{asked}", vocab::SIGNALS));
                let signal = graph.number(&node).ok_or(format!("this car has no signal {asked}"))?;
                let checked = graph.object(signal, observes);
                let assumed = by_default.and_then(|property| graph.object(signal, property));
                found.push((signal, checked.or(assumed), checked.is_none() && assumed.is_some()));
            } else {
                // A quantity: the signals known to observe it and, only when there are none,
                // those that observe it by default.
                let quantity = number(&iri(&format!("{}{asked}", vocab::JEDGE)))?;
                found.extend(graph.subjects(observes, quantity).map(|signal| (signal, Some(quantity), false)));
                if found.is_empty() {
                    if let Some(property) = by_default {
                        found.extend(graph.subjects(property, quantity).map(|signal| (signal, Some(quantity), true)));
                    }
                }
                if found.is_empty() {
                    return Err(format!("no signal of this car observes {asked}"));
                }
            }
            for (signal, quantity, assumed) in found {
                let slot = live.slot_of(signal).ok_or(format!("{} is not a signal", graph.term(signal)))?;
                // A signal asked for twice is read once.
                if signals.iter().any(|member| member.slot == slot) {
                    continue;
                }
                // What it is: the label of its quantity or, without one, its own.
                let described = quantity
                    .and_then(|q| text_at(q, label))
                    .or_else(|| text_at(signal, label))
                    .unwrap_or_else(|| "undescribed signal".to_string());
                let in_unit = quantity.and_then(|q| text_at(q, unit)).map_or(String::new(), |u| format!(", in {u}"));
                let note = if assumed { ", assumed" } else { "" };
                any_assumed |= assumed;
                let message = graph.object(signal, in_message);
                let message_name = message.and_then(|m| text_at(m, name)).unwrap_or_default();
                let sender = message.and_then(|m| graph.object(m, sent_by)).and_then(|ecu| text_at(ecu, name));
                let from = sender.map_or(String::new(), |ecu| format!(", from ECU {ecu}"));
                let step = text_at(signal, resolution).and_then(|r| r.parse().ok()).unwrap_or(1.0);
                signals.push(Member { slot, quantity, decimals: decimals(step) });
                let signal_name = text_at(signal, name).unwrap_or_default();
                let _ = if bare {
                    writeln!(prefix, "{}. {message_name}.{signal_name}", signals.len())
                } else {
                    writeln!(prefix, "{}. {described}{in_unit} ({message_name}.{signal_name}{from}{note})", signals.len())
                };
            }
        }
        if any_assumed && !bare {
            prefix.push_str("Assumed: what that signal is was read from its name and unit, and not yet checked against its values.\n");
        }

        // The edges: every equation of the graph that only involves quantities of the question.
        let mut relations = String::new();
        if let Some(equation) = graph.number(vocab::EQUATION) {
            let over = number(vocab::OVER)?;
            for relation in graph.subjects(a, equation) {
                let involved: Vec<u32> = graph.objects(relation, over).collect();
                if involved.is_empty() || !involved.iter().all(|q| signals.iter().any(|m| m.quantity == Some(*q))) {
                    continue;
                }
                let ends: Vec<String> = signals
                    .iter()
                    .enumerate()
                    .filter(|(_, member)| member.quantity.is_some_and(|q| involved.contains(&q)))
                    .map(|(i, _)| (i + 1).to_string())
                    .collect();
                let described = text_at(relation, label).unwrap_or_default();
                let _ = writeln!(relations, "- {described} ({})", ends.join(", "));
            }
        }
        if !relations.is_empty() && !bare {
            prefix.push_str("Relations between them:\n");
            prefix.push_str(&relations);
        }
        let options: Vec<&str> = options.split(',').map(str::trim).collect();
        let _ = writeln!(prefix, "Question: {text}");
        let _ = writeln!(prefix, "Options: {}", options.join(", "));
        prefix.push_str("Each window gives, for every signal: last value, minimum, maximum, mean and change (last minus first).\n");

        // The first window ends once a whole window of stream time has gone by.
        let next_end = (window.div_ceil(slide) * slide) as i64;
        Ok(Question { id: id.to_string(), window, slide, next_end, signals, prefix })
    }

    /// Writes what the signals did in the window that ends at bucket `end`: the part of the
    /// prompt that changes.
    pub fn window_text(&self, live: &Live, end: i64, out: &mut String) {
        let (seconds, until) = (self.window as f64 * live.bucket_s, end as f64 * live.bucket_s);
        let _ = writeln!(out, "Window of {seconds} s ending at t={until} s:");
        for (i, member) in self.signals.iter().enumerate() {
            let (n, d) = (i + 1, member.decimals);
            let _ = match live.summary(member.slot, end, self.window) {
                Some(s) => writeln!(
                    out,
                    "{n}. last {:.d$}, min {:.d$}, max {:.d$}, mean {:.d$}, change {:+.d$}",
                    tidy(s.last, d),
                    tidy(s.min, d),
                    tidy(s.max, d),
                    tidy(s.mean(), d),
                    tidy(s.change(), d)
                ),
                None => writeln!(out, "{n}. no data"),
            };
        }
        out.push_str("Answer:");
    }

    /// Writes how each signal moved through that window: for every signal, in the order of the
    /// text, its mean in each bucket of the window, from the oldest to the newest. It is a JSON
    /// array with one array per signal. Each mean is written as text, with the decimals of its
    /// signal, so that whoever shows it shows the digits `window_text` would; a bucket in which
    /// nothing arrived is `null`.
    pub fn window_series(&self, live: &Live, end: i64, out: &mut String) {
        out.push('[');
        for (i, member) in self.signals.iter().enumerate() {
            out.push_str(if i == 0 { "[" } else { ",[" });
            let d = member.decimals;
            for (j, mean) in live.series(member.slot, end, self.window).enumerate() {
                if j > 0 {
                    out.push(',');
                }
                let _ = match mean {
                    Some(mean) => write!(out, "\"{:.d$}\"", tidy(mean, d)),
                    None => write!(out, "null"),
                };
            }
            out.push(']');
        }
        out.push(']');
    }

    /// The same window as RDF: a node for the window and one observation per signal that had
    /// values, each triple as three terms in N-Triples syntax. `serial` keeps the blank nodes of
    /// this window apart from those of every other one.
    pub fn window_triples(&self, graph: &Graph, live: &Live, end: i64, serial: u64) -> Vec<[String; 3]> {
        let number = |value: f64, kind: &str| literal(&value.to_string(), kind);
        let window = format!("_:w{serial}");
        let mut triples = Vec::new();
        let mut add = |subject: &str, predicate: &str, object: String| {
            triples.push([subject.to_string(), predicate.to_string(), object]);
        };
        add(&window, vocab::TYPE, vocab::WINDOW.to_string());
        add(&window, vocab::QUESTION, literal(&self.id, ""));
        add(&window, vocab::ENDS_AT, number(end as f64 * live.bucket_s, vocab::DECIMAL));
        add(&window, vocab::SECONDS, number(self.window as f64 * live.bucket_s, vocab::DECIMAL));
        for member in &self.signals {
            let Some(s) = live.summary(member.slot, end, self.window) else {
                continue;
            };
            let observation = format!("{window}s{}", member.slot);
            add(&observation, vocab::TYPE, vocab::OBSERVATION.to_string());
            add(&observation, vocab::IN_WINDOW, window.clone());
            add(&observation, vocab::MADE_BY_SENSOR, graph.term(live.node(member.slot)).to_string());
            if let Some(quantity) = member.quantity {
                add(&observation, vocab::OBSERVED_PROPERTY, graph.term(quantity).to_string());
            }
            add(&observation, vocab::HAS_SIMPLE_RESULT, number(s.last, vocab::DOUBLE));
            add(&observation, vocab::STREAM_TIME, number(s.last_t, vocab::DECIMAL));
            add(&observation, vocab::SAMPLES, literal(&s.count.to_string(), vocab::INTEGER));
            add(&observation, vocab::WINDOW_FIRST, number(s.first, vocab::DOUBLE));
            add(&observation, vocab::WINDOW_MIN, number(s.min, vocab::DOUBLE));
            add(&observation, vocab::WINDOW_MAX, number(s.max, vocab::DOUBLE));
            add(&observation, vocab::WINDOW_MEAN, number(tidy(s.mean(), 6), vocab::DOUBLE));
        }
        triples
    }
}

/// How many decimals show a value at the resolution of its signal, three at most.
fn decimals(resolution: f64) -> usize {
    (0..3).find(|&d| resolution * 10f64.powi(d) >= 0.999).unwrap_or(3) as usize
}

/// A value rounded to that many decimals. Adding zero turns a negative zero into a plain one.
fn tidy(value: f64, decimals: usize) -> f64 {
    let scale = 10f64.powi(decimals as i32);
    (value * scale).round() / scale + 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Sample;

    /// Six signals in one message sent by one ECU. Two observe a quantity, and an equation ties
    /// those two quantities. Two more observe a quantity only by default, one is described
    /// without a quantity and of the last one the graph says nothing.
    fn graph() -> Graph {
        let j = vocab::JEDGE;
        let mut text = String::new();
        let mut add = |s: &str, p: &str, o: &str| text.push_str(&format!("{s} {p} {o} .\n"));
        let node = |signal: &str| format!("<{}MOTION.{signal}>", vocab::SIGNALS);
        let steps = [("SPEED", "0.5"), ("ACCEL", "0.01"), ("SPEED_COPY", "1"), ("TEMP", "0.75"), ("PARITY", "1"), ("ALIVE", "1")];
        for (signal, step) in steps {
            add(&node(signal), vocab::TYPE, vocab::SIGNAL);
            add(&node(signal), vocab::NAME, &literal(signal, ""));
            add(&node(signal), vocab::IN_MESSAGE, "<https://e.org/m>");
            add(&node(signal), vocab::RESOLUTION, &literal(step, vocab::DOUBLE));
        }
        add(&node("SPEED"), vocab::OBSERVES, &format!("<{j}vehicle_speed>"));
        add(&node("ACCEL"), vocab::OBSERVES, &format!("<{j}longitudinal_acceleration>"));
        add(&node("SPEED_COPY"), vocab::OBSERVES_BY_DEFAULT, &format!("<{j}vehicle_speed>"));
        add(&node("TEMP"), vocab::OBSERVES_BY_DEFAULT, &format!("<{j}coolant_temperature>"));
        add(&node("PARITY"), vocab::LABEL, "\"parity bit\"@en");
        add(&format!("<{j}coolant_temperature>"), vocab::LABEL, "\"engine coolant temperature\"@en");
        add(&format!("<{j}coolant_temperature>"), vocab::UNIT, "\"degC\"");
        add("<https://e.org/m>", vocab::NAME, "\"MOTION\"");
        add("<https://e.org/m>", vocab::SENT_BY, "<https://e.org/esc>");
        add("<https://e.org/esc>", vocab::NAME, "\"ESC\"");
        add(&format!("<{j}vehicle_speed>"), vocab::LABEL, "\"vehicle speed\"@en");
        add(&format!("<{j}vehicle_speed>"), vocab::UNIT, "\"km/h\"");
        add(&format!("<{j}longitudinal_acceleration>"), vocab::LABEL, "\"longitudinal acceleration\"@en");
        add(&format!("<{j}longitudinal_acceleration>"), vocab::UNIT, "\"m/s^2\"");
        add(&format!("<{j}speed_eq>"), vocab::TYPE, vocab::EQUATION);
        add(&format!("<{j}speed_eq>"), vocab::LABEL, "\"the rate of change of the speed is the acceleration\"@en");
        add(&format!("<{j}speed_eq>"), vocab::OVER, &format!("<{j}vehicle_speed>"));
        add(&format!("<{j}speed_eq>"), vocab::OVER, &format!("<{j}longitudinal_acceleration>"));
        Graph::from_text(&text)
    }

    const LINE: &str = "braking\t2\t2\tvehicle_speed, longitudinal_acceleration\tIs the vehicle braking?\tyes, no";

    #[test]
    fn the_prefix_is_the_subgraph_of_the_question() {
        let graph = graph();
        let live = Live::new(&graph, 1.0, 4).unwrap();
        let question = Question::register(LINE, &graph, &live).unwrap();
        assert_eq!((question.window, question.slide, question.next_end), (2, 2, 2));
        assert_eq!(
            question.prefix,
            "Signals of the vehicle:\n\
             1. vehicle speed, in km/h (MOTION.SPEED, from ECU ESC)\n\
             2. longitudinal acceleration, in m/s^2 (MOTION.ACCEL, from ECU ESC)\n\
             Relations between them:\n\
             - the rate of change of the speed is the acceleration (1, 2)\n\
             Question: Is the vehicle braking?\n\
             Options: yes, no\n\
             Each window gives, for every signal: last value, minimum, maximum, mean and change (last minus first).\n"
        );

        // Without what the graph knows: the same two signals by their DBC names, and no relation.
        let bare = Question::register_as(LINE, &graph, &live, true).unwrap();
        assert_eq!(
            bare.prefix,
            "Signals of the vehicle:\n\
             1. MOTION.SPEED\n\
             2. MOTION.ACCEL\n\
             Question: Is the vehicle braking?\n\
             Options: yes, no\n\
             Each window gives, for every signal: last value, minimum, maximum, mean and change (last minus first).\n"
        );
        assert_eq!(bare.signals.len(), question.signals.len());

        // With only one of its two quantities, the equation is left out.
        let alone = LINE.replace("vehicle_speed, longitudinal_acceleration", "vehicle_speed");
        assert!(!Question::register(&alone, &graph, &live).unwrap().prefix.contains("Relations"));
    }

    #[test]
    fn a_question_the_graph_cannot_answer_is_refused() {
        let graph = graph();
        let live = Live::new(&graph, 1.0, 4).unwrap();
        let refused = |line: &str| Question::register(line, &graph, &live).err().unwrap();
        assert!(refused("only\ttwo fields").contains("6 fields"));
        assert!(refused(&LINE.replace("vehicle_speed,", "yaw_rate,")).contains("yaw_rate"));
        assert!(refused(&LINE.replace("\t2\t2\t", "\t9\t2\t")).contains("does not fit"));
        assert!(refused(&LINE.replace("\t2\t2\t", "\t2\t0.1\t")).contains("shorter than one bucket"));
    }

    #[test]
    fn a_window_as_text_and_as_triples() {
        let graph = graph();
        let mut live = Live::new(&graph, 1.0, 4).unwrap();
        let question = Question::register(LINE, &graph, &live).unwrap();
        for (t, speed) in [(0.1, 50.0), (0.9, 49.5), (1.5, 44.0)] {
            let signals = vec![("SPEED".to_string(), speed)];
            live.accept(&Sample { t, id: 1, message: "MOTION".to_string(), signals });
        }
        let mut text = String::new();
        question.window_text(&live, 2, &mut text);
        assert_eq!(
            text,
            "Window of 2 s ending at t=2 s:\n\
             1. last 44.0, min 44.0, max 50.0, mean 47.8, change -6.0\n\
             2. no data\n\
             Answer:"
        );

        // How each signal moved: its mean in each of the two seconds of the window.
        let mut series = String::new();
        question.window_series(&live, 2, &mut series);
        assert_eq!(series, "[[\"49.8\",\"44.0\"],[null,null]]");

        let triples = question.window_triples(&graph, &live, 2, 7);
        assert_eq!(triples.len(), 4 + 11); // the window, and one observation: the signal that had values
        assert_eq!(triples[1], ["_:w7".to_string(), vocab::QUESTION.to_string(), "\"braking\"".to_string()]);
        let result = triples.iter().find(|t| t[1] == vocab::HAS_SIMPLE_RESULT).unwrap();
        assert_eq!(result[2], "\"44\"^^<http://www.w3.org/2001/XMLSchema#double>");
        assert!(result[0].starts_with("_:w7s"));
    }

    #[test]
    fn default_quantities_and_single_signals_can_be_asked_for() {
        let graph = graph();
        let mut live = Live::new(&graph, 1.0, 4).unwrap();
        let ask = |asked: &str| {
            Question::register(&LINE.replace("vehicle_speed, longitudinal_acceleration", asked), &graph, &live)
        };
        let lines = |question: &Question, count: usize| -> Vec<String> {
            question.prefix.lines().take(count).map(str::to_string).collect()
        };

        // A quantity with a checked signal keeps to it. One without takes the signals that
        // observe it by default, and the prefix says that they are assumed.
        let question = ask("vehicle_speed, coolant_temperature").unwrap();
        assert_eq!(
            lines(&question, 4),
            [
                "Signals of the vehicle:",
                "1. vehicle speed, in km/h (MOTION.SPEED, from ECU ESC)",
                "2. engine coolant temperature, in degC (MOTION.TEMP, from ECU ESC, assumed)",
                "Assumed: what that signal is was read from its name and unit, and not yet checked against its values.",
            ]
        );

        // A signal asked for by name: one that observes by default, one the graph only
        // describes and one it says nothing about. A signal asked for twice is read once.
        let question = ask("vehicle_speed, MOTION.SPEED_COPY, MOTION.SPEED, MOTION.PARITY, MOTION.ALIVE").unwrap();
        assert_eq!(
            lines(&question, 5)[1..],
            [
                "1. vehicle speed, in km/h (MOTION.SPEED, from ECU ESC)",
                "2. vehicle speed, in km/h (MOTION.SPEED_COPY, from ECU ESC, assumed)",
                "3. parity bit (MOTION.PARITY, from ECU ESC)",
                "4. undescribed signal (MOTION.ALIVE, from ECU ESC)",
            ]
        );
        assert!(ask("MOTION.NOTHING").err().unwrap().contains("no signal MOTION.NOTHING"));

        // An assumed quantity takes part in the equations like any other.
        assert!(ask("MOTION.SPEED_COPY, longitudinal_acceleration").unwrap().prefix.contains("(1, 2)"));

        // As RDF, a signal with no quantity gives an observation with no observed property.
        let question = ask("MOTION.PARITY").unwrap();
        let signals = vec![("PARITY".to_string(), 1.0)];
        live.accept(&Sample { t: 0.5, id: 1, message: "MOTION".to_string(), signals });
        let triples = question.window_triples(&graph, &live, 2, 1);
        assert_eq!(triples.len(), 4 + 10);
        assert!(!triples.iter().any(|t| t[1] == vocab::OBSERVED_PROPERTY));
    }

    #[test]
    fn decimals_follow_the_resolution() {
        let shown: Vec<usize> = [4.0, 1.0, 0.5, 0.1, 0.03125, 0.01, 0.0078125, 0.0001].map(decimals).to_vec();
        assert_eq!(shown, [0, 0, 1, 1, 2, 2, 3, 3]);
        assert_eq!(format!("{:.1}", tidy(-0.04, 1)), "0.0");
    }
}
