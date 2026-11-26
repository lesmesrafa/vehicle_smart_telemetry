//! Gateway side: waits for the vehicle, receives the decoded signals and keeps the latest value
//! of each one. With `--graph`, every value also goes into the knowledge graph, and each
//! registered question gets, window after window, the part of the graph it needs.

use vehicle_smart_telemetry::graph::Graph;
use vehicle_smart_telemetry::jelly;
use vehicle_smart_telemetry::live::Live;
use vehicle_smart_telemetry::metrics::{cpu_seconds, memory_mb};
use vehicle_smart_telemetry::ntriples::literal;
use vehicle_smart_telemetry::options::{fail, Args};
use vehicle_smart_telemetry::question::Question;
use vehicle_smart_telemetry::timing::Delays;
use vehicle_smart_telemetry::wire::{self, Sample};
use vehicle_smart_telemetry::{with_name, DEFAULT_PORT};
use std::collections::HashMap;
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::net::TcpListener;
use std::time::Instant;

const HELP: &str = "\
signal_receiver: receives the decoded signals the vehicle sends and reports what arrives.

use:  signal_receiver
      signal_receiver --watch WHL_SPD11.WHL_SPD_FL,ESP12.LONG_ACCEL
      signal_receiver --save received.ndjson --quiet
      signal_receiver --graph signal_graph.nt --questions questions.tsv --prompts prompts.ndjson

  --listen ADDR:PORT   where to listen (default 0.0.0.0:7700, every network interface)
  --save FILE          also write every line received to FILE, unchanged
  --watch LIST         signals to show in each report, as MESSAGE.SIGNAL separated by commas
  --every S            seconds between reports (1)
  --quiet              no reports, only the final summary

the knowledge graph:
  --graph FILE         fixed part of the graph, as N-Triples (written by graph_builder)
  --questions FILE     questions to register, one per line (see questions.tsv)
  --bucket S           seconds of stream time each bucket of a signal covers (1)
  --history S          seconds of history kept per signal: the longest window (10)
  --prompts FILE       write what a language model would read: each prefix once, then each window
  --bare               in those prefixes, leave out what the graph knows: only the DBC name of each signal
  --rdf FILE           write each window as RDF observations, in N-Triples
  --jelly FILE         write each window as RDF observations, in Jelly";
const WITH_VALUE: [&str; 11] = [
    "--listen", "--save", "--watch", "--every", "--graph", "--questions", "--bucket", "--history", "--prompts", "--rdf",
    "--jelly",
];
const WITHOUT_VALUE: [&str; 2] = ["--quiet", "--bare"];

/// What the gateway knows from what has arrived.
struct State {
    /// Latest value of every signal: message name -> signal name -> value.
    latest: HashMap<String, HashMap<String, f64>>,
    /// How many different signals have arrived.
    distinct: usize,
    messages: u64,
    values: u64,
    /// Recording time of the last message.
    recording_s: f64,
    /// Smallest gap seen between the arrival of a message and its recording time.
    base_s: f64,
}

impl State {
    fn new() -> State {
        State { latest: HashMap::new(), distinct: 0, messages: 0, values: 0, recording_s: 0.0, base_s: f64::INFINITY }
    }

    /// Takes one decoded CAN message: counts it and keeps the latest value of each signal.
    ///
    /// Returns the timing error of the message: how much later it arrived than the recording says,
    /// taking as zero the message that has travelled fastest so far.
    fn accept(&mut self, sample: Sample, arrival_s: f64) -> f64 {
        self.messages += 1;
        self.values += sample.signals.len() as u64;
        self.recording_s = sample.t;
        let gap = arrival_s - sample.t;
        self.base_s = self.base_s.min(gap);

        let signals = self.latest.entry(sample.message).or_default();
        for (name, value) in sample.signals {
            if signals.insert(name, value).is_none() {
                self.distinct += 1;
            }
        }
        gap - self.base_s
    }

    /// The latest value of a signal, or `None` if it has not arrived yet.
    fn value(&self, message: &str, signal: &str) -> Option<f64> {
        self.latest.get(message)?.get(signal).copied()
    }
}

/// The knowledge graph of the gateway: its fixed part, its live part and the registered
/// questions, with the files their windows go to.
struct Knowledge {
    graph: Graph,
    live: Live,
    questions: Vec<Question>,
    /// How many windows each question has had.
    produced: Vec<u64>,
    prompts: Option<BufWriter<File>>,
    rdf: Option<BufWriter<File>>,
    jelly: Option<jelly::Writer<BufWriter<File>>>,
    /// The text of the window being produced. It is reused, so no window reserves memory.
    text: String,
    /// Whether the first window of each question is shown on screen.
    show: bool,
    windows: u64,
    window_bytes: u64,
    triples: u64,
    rdf_bytes: u64,
}

impl Knowledge {
    /// Loads the graph given with `--graph` and registers the questions. Without `--graph`
    /// there is no knowledge graph and the receiver works as before.
    fn open(a: &Args, show: bool) -> Result<Option<Knowledge>, Box<dyn Error>> {
        let Some(path) = a.optional("--graph") else {
            if let Some(option) = ["--questions", "--prompts", "--rdf", "--jelly"].into_iter().find(|o| a.optional(o).is_some()) {
                return Err(format!("{option} needs --graph").into());
            }
            return Ok(None);
        };
        let create = |option: &str| -> io::Result<Option<BufWriter<File>>> {
            match a.optional(option) {
                Some(path) => Ok(Some(BufWriter::new(File::create(path).map_err(|e| with_name(path, e))?))),
                None => Ok(None),
            }
        };
        let graph = Graph::load(path)?;
        let bucket_s = a.number("--bucket", 1.0);
        if bucket_s <= 0.0 {
            return Err("--bucket must be more than zero seconds".into());
        }
        let ring = (a.number("--history", 10.0) / bucket_s).ceil().max(1.0) as usize;
        let live = Live::new(&graph, bucket_s, ring)?;
        println!(
            "knowledge graph: {} triples and {} terms from {path}; {} signals, each with {ring} buckets of {bucket_s} s",
            graph.triples(),
            graph.terms(),
            live.slots()
        );

        let mut questions = Vec::new();
        let mut prompts = create("--prompts")?;
        if let Some(path) = a.optional("--questions") {
            let text = fs::read_to_string(path).map_err(|e| with_name(path, e))?;
            for line in text.lines().filter(|line| !line.trim().is_empty() && !line.starts_with('#')) {
                let question = Question::register_as(line, &graph, &live, a.enabled("--bare")).map_err(|e| format!("{path}: {e}"))?;
                println!(
                    "question '{}': {} signals, a window of {} s every {} s, prefix of {} bytes",
                    question.id,
                    question.signals.len(),
                    question.window as f64 * bucket_s,
                    question.slide as f64 * bucket_s,
                    question.prefix.len()
                );
                // A literal with no language or datatype is written just like a JSON string.
                if let Some(file) = prompts.as_mut() {
                    writeln!(file, "{{\"question\":{},\"prefix\":{}}}", literal(&question.id, ""), literal(&question.prefix, ""))?;
                }
                questions.push(question);
            }
        }
        // The prefixes are written out now, so that a model that follows the file reads them
        // before the first window comes.
        if let Some(file) = prompts.as_mut() {
            file.flush()?;
        }
        Ok(Some(Knowledge {
            produced: vec![0; questions.len()],
            graph,
            live,
            questions,
            prompts,
            rdf: create("--rdf")?,
            jelly: create("--jelly")?.map(jelly::Writer::new),
            text: String::new(),
            show,
            windows: 0,
            window_bytes: 0,
            triples: 0,
            rdf_bytes: 0,
        }))
    }

    /// Takes one decoded CAN message. First it produces every window that ended before this
    /// message, then the values go into their slots.
    fn accept(&mut self, sample: &Sample) -> io::Result<()> {
        let now = self.live.bucket_of(sample.t);
        for i in 0..self.questions.len() {
            while now >= self.questions[i].next_end {
                let end = self.questions[i].next_end;
                self.questions[i].next_end += self.questions[i].slide as i64;
                self.window(i, end)?;
            }
        }
        self.live.accept(sample);
        Ok(())
    }

    /// Produces the window of question `i` that ends at bucket `end`, in every form asked for.
    fn window(&mut self, i: usize, end: i64) -> io::Result<()> {
        let question = &self.questions[i];
        self.text.clear();
        question.window_text(&self.live, end, &mut self.text);
        if self.show && self.produced[i] == 0 {
            println!("--- what a model reads for '{}' ---\n{}{}\n---", question.id, question.prefix, self.text);
        }
        if let Some(file) = self.prompts.as_mut() {
            let (id, t) = (literal(&question.id, ""), end as f64 * self.live.bucket_s);
            writeln!(file, "{{\"question\":{id},\"t\":{t},\"window\":{}}}", literal(&self.text, ""))?;
            // Written out at once: a model that follows this file answers while the car is still sending.
            file.flush()?;
        }
        if self.rdf.is_some() || self.jelly.is_some() {
            let triples = question.window_triples(&self.graph, &self.live, end, self.windows);
            for [s, p, o] in &triples {
                if let Some(file) = self.rdf.as_mut() {
                    writeln!(file, "{s} {p} {o} .")?;
                    self.rdf_bytes += (s.len() + p.len() + o.len() + 5) as u64;
                }
                if let Some(writer) = self.jelly.as_mut() {
                    writer.triple([s, p, o])?;
                }
            }
            if let Some(writer) = self.jelly.as_mut() {
                writer.end_graph()?;
            }
            self.triples += triples.len() as u64;
        }
        self.produced[i] += 1;
        self.windows += 1;
        self.window_bytes += self.text.len() as u64;
        Ok(())
    }

    /// Writes to disk what the files still hold in memory.
    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = self.prompts.as_mut() {
            file.flush()?;
        }
        if let Some(file) = self.rdf.as_mut() {
            file.flush()?;
        }
        if let Some(writer) = self.jelly.as_mut() {
            writer.flush()?;
        }
        Ok(())
    }

    fn summary(&self) {
        println!(
            "knowledge graph: {} values written into {} signals; {} values of signals the graph does not have",
            self.live.written,
            self.live.slots(),
            self.live.unknown
        );
        for (question, produced) in self.questions.iter().zip(&self.produced) {
            println!("question '{}': {produced} windows", question.id);
        }
        if self.windows > 0 {
            let prefixes: usize = self.questions.iter().map(|q| q.prefix.len()).sum();
            println!(
                "text for a model: {prefixes} bytes of prefixes, read once; {} windows, {:.0} bytes each on average",
                self.windows,
                self.window_bytes as f64 / self.windows as f64
            );
        }
        if self.triples > 0 {
            let jelly = self.jelly.as_ref().map_or(String::new(), |w| format!("; Jelly: {} bytes in {} frames", w.bytes, w.frames));
            let rdf = if self.rdf.is_some() { format!("; N-Triples: {} bytes", self.rdf_bytes) } else { String::new() };
            println!("windows as RDF: {} triples{rdf}{jelly}", self.triples);
        }
    }
}

fn main() {
    if let Err(error) = run() {
        fail(&error.to_string());
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let a = Args::parse(HELP, &WITH_VALUE, &WITHOUT_VALUE);
    let every = a.number("--every", 1.0);
    let quiet = a.enabled("--quiet");
    let mut watched: Vec<(&str, &str)> = Vec::new();
    for key in a.optional("--watch").unwrap_or("").split(',').filter(|key| !key.is_empty()) {
        watched.push(key.split_once('.').ok_or(format!("--watch: '{key}' does not have the form MESSAGE.SIGNAL"))?);
    }
    let mut save = match a.optional("--save") {
        Some(path) => Some(BufWriter::new(File::create(path).map_err(|e| with_name(path, e))?)),
        None => None,
    };

    let mut knowledge = Knowledge::open(&a, !quiet)?;

    let address = a.text("--listen", &format!("0.0.0.0:{DEFAULT_PORT}"));
    let listener = TcpListener::bind(&address).map_err(|e| with_name(&address, e))?;
    println!("signal_receiver: listening on {address}, waiting for the vehicle");
    let (stream, peer) = listener.accept()?;
    println!("vehicle connected from {peer}");

    let mut reader = BufReader::new(stream);
    let mut state = State::new();
    let mut line = String::new();
    let mut unreadable = 0u64;
    let clock = Instant::now();
    // Timing error of every message, and the worst one since the last report.
    let (mut errors, mut error_s) = (Delays::default(), 0.0f64);
    let (mut t_report, mut messages_before, mut values_before) = (0.0f64, 0u64, 0u64);

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break, // the vehicle closed the connection: the recording has ended
            Ok(_) => {}
            Err(error) => {
                println!("the connection broke: {error}");
                break;
            }
        }
        let now = clock.elapsed().as_secs_f64();
        if let Some(file) = save.as_mut() {
            file.write_all(line.as_bytes())?;
        }
        match wire::decode(&line) {
            Some(sample) => {
                if let Some(knowledge) = knowledge.as_mut() {
                    knowledge.accept(&sample)?;
                }
                let error = state.accept(sample, now);
                errors.add(error);
                error_s = error_s.max(error);
            }
            None => unreadable += 1,
        }

        if now - t_report >= every {
            if !quiet {
                let seconds = now - t_report;
                let shown: Vec<String> = watched
                    .iter()
                    .map(|&(message, signal)| match state.value(message, signal) {
                        Some(value) => format!("{signal}={value:.2}"),
                        None => format!("{signal}=-"),
                    })
                    .collect();
                println!(
                    "t={:7.1} s | {:5.0} messages/s | {:6.0} values/s | {:4} signals | timing error up to {:5.2} ms | {}",
                    state.recording_s,
                    (state.messages - messages_before) as f64 / seconds,
                    (state.values - values_before) as f64 / seconds,
                    state.distinct,
                    1000.0 * error_s,
                    shown.join(" ")
                );
            }
            if let Some(file) = save.as_mut() {
                file.flush()?;
            }
            if let Some(knowledge) = knowledge.as_mut() {
                knowledge.flush()?;
            }
            error_s = 0.0;
            (t_report, messages_before, values_before) = (now, state.messages, state.values);
        }
    }

    if let Some(file) = save.as_mut() {
        file.flush()?;
    }
    println!("\n--- signal_receiver summary ---");
    println!("messages received: {} with {} signal values", state.messages, state.values);
    println!("different messages: {}; different signals: {}", state.latest.len(), state.distinct);
    println!(
        "recording time covered: {:.1} s, received in {:.1} s",
        state.recording_s,
        clock.elapsed().as_secs_f64()
    );
    println!("timing error: {} (only meaningful with the sender at speed 1)", errors.describe());
    if unreadable > 0 {
        println!("lines that could not be read: {unreadable}");
    }
    if let Some(knowledge) = knowledge.as_mut() {
        knowledge.flush()?;
        knowledge.summary();
    }
    println!("cost of this process: {:.2} s of CPU, peak memory {:.1} MB", cpu_seconds(), memory_mb().1);
    Ok(())
}
