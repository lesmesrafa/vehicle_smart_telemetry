//! Gateway side: waits for the vehicle, receives the decoded signals and keeps the latest value
//! of each one. The knowledge graph will be fed from `State::accept`.

use vehicle_smart_telemetry::metrics::{cpu_seconds, memory_mb};
use vehicle_smart_telemetry::options::{fail, Args};
use vehicle_smart_telemetry::timing::Delays;
use vehicle_smart_telemetry::wire::{self, Sample};
use vehicle_smart_telemetry::{with_name, DEFAULT_PORT};
use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::net::TcpListener;
use std::time::Instant;

const HELP: &str = "\
signal_receiver: receives the decoded signals the vehicle sends and reports what arrives.

use:  signal_receiver
      signal_receiver --watch WHL_SPD11.WHL_SPD_FL,ESP12.LONG_ACCEL
      signal_receiver --save received.ndjson --quiet

  --listen ADDR:PORT   where to listen (default 0.0.0.0:7700, every network interface)
  --save FILE          also write every line received to FILE, unchanged
  --watch LIST         signals to show in each report, as MESSAGE.SIGNAL separated by commas
  --every S            seconds between reports (1)
  --quiet              no reports, only the final summary";
const WITH_VALUE: [&str; 4] = ["--listen", "--save", "--watch", "--every"];
const WITHOUT_VALUE: [&str; 1] = ["--quiet"];

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

    /// Takes one decoded CAN message. THE KNOWLEDGE GRAPH WILL BE FED FROM HERE.
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
    println!("cost of this process: {:.2} s of CPU, peak memory {:.1} MB", cpu_seconds(), memory_mb().1);
    Ok(())
}
