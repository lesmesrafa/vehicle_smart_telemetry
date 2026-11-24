//! Vehicle side: reads a CAN recording, decodes every frame with the DBC and sends the decoded
//! signals to the gateway at the pace of the recording.

use vehicle_smart_telemetry::candump::parse_line;
use vehicle_smart_telemetry::dbc::Dbc;
use vehicle_smart_telemetry::options::{fail, Args};
use vehicle_smart_telemetry::timing::Delays;
use vehicle_smart_telemetry::{wire, with_name, DEFAULT_PORT};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, ErrorKind, Write};
use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant};

const HELP: &str = "\
signal_sender: decodes a CAN recording and streams its signals to the gateway in real time.

use:  signal_sender --log data/dump1.log
      signal_sender --log data/dump1.log --to 10.77.0.1:7700 --duration 60
      signal_sender --log data/dump1.log --stdout --speed 0 > expected.ndjson

  --log FILE       recording in candump format, one frame per line: (time) bus ID#DATA (required)
  --dbc FILE       DBC file (default hyundai_2015_ccan.dbc)
  --to HOST:PORT   where the gateway listens (default 10.77.0.1:7700)
  --speed X        1 is real time, 2 is twice as fast, 0 does not wait at all (default 1)
  --duration S     send only the first S seconds of the recording
  --stdout         write the lines to standard output instead of sending them";
const WITH_VALUE: [&str; 5] = ["--log", "--dbc", "--to", "--speed", "--duration"];
const WITHOUT_VALUE: [&str; 1] = ["--stdout"];

/// Seconds of recording between two progress lines.
const REPORT_EVERY_S: f64 = 5.0;

/// What has been read and sent so far.
#[derive(Default)]
struct Totals {
    frames: u64,
    not_frames: u64,
    messages: u64,
    values: u64,
    sent_ids: BTreeSet<u32>,
    unknown_ids: BTreeMap<u32, u64>,
    recording_s: f64,
    /// How long after its moment in the recording each frame left.
    late: Delays,
}

fn main() {
    if let Err(error) = run() {
        fail(&error.to_string());
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let a = Args::parse(HELP, &WITH_VALUE, &WITHOUT_VALUE);
    let log_path = a.optional("--log").ok_or("missing --log FILE; try --help")?;
    let speed = a.number("--speed", 1.0);
    if !speed.is_finite() || speed < 0.0 {
        return Err("--speed must be 0 or a positive number".into());
    }
    let limit = a.optional("--duration").map(|_| a.number("--duration", 0.0));

    let dbc_path = a.text("--dbc", "hyundai_2015_ccan.dbc");
    let dbc = Dbc::load(&dbc_path)?;
    // The wire format writes the names as they are, so they must be plain identifiers.
    for message in &dbc.messages {
        let mut names = std::iter::once(&message.name).chain(message.signals.iter().map(|s| &s.name));
        if let Some(name) = names.find(|name| !wire::is_plain(name)) {
            return Err(format!("{dbc_path}: the name '{name}' has characters the wire format does not accept").into());
        }
    }
    let n_signals: usize = dbc.messages.iter().map(|m| m.signals.len()).sum();
    if n_signals == 0 {
        return Err(format!("{dbc_path} defines no signals").into());
    }
    let log = BufReader::new(File::open(log_path).map_err(|e| with_name(log_path, e))?);
    eprintln!("signal_sender: {dbc_path} defines {} messages and {n_signals} signals", dbc.messages.len());

    // Everything this program reports goes to standard error, so that with --stdout the
    // standard output carries only the data.
    let mut out: Box<dyn Write> = if a.enabled("--stdout") {
        Box::new(BufWriter::new(io::stdout().lock()))
    } else {
        Box::new(BufWriter::new(connect(&a.text("--to", &format!("10.77.0.1:{DEFAULT_PORT}")))?))
    };
    if speed == 1.0 {
        eprintln!("sending {log_path} in real time");
    } else {
        eprintln!("sending {log_path} at speed {speed} (1 is real time)");
    }

    let mut totals = Totals::default();
    let clock = Instant::now();
    let result = stream(log, &dbc, out.as_mut(), speed, limit, &mut totals);
    let took_s = clock.elapsed().as_secs_f64();
    drop(out); // closes the connection: that is how the gateway knows the recording has ended

    eprintln!("\n--- signal_sender summary ---");
    eprintln!("frames read: {} in {:.1} s of recording", totals.frames, totals.recording_s);
    eprintln!(
        "messages sent: {} with {} signal values, from {} identifiers",
        totals.messages,
        totals.values,
        totals.sent_ids.len()
    );
    if !totals.unknown_ids.is_empty() {
        let not_sent: u64 = totals.unknown_ids.values().sum();
        let list: Vec<String> = totals.unknown_ids.iter().map(|(id, n)| format!("0x{id:03X} x{n}")).collect();
        eprintln!("frames not sent because the DBC does not have their identifier: {not_sent} ({})", list.join(", "));
    }
    if totals.not_frames > 0 {
        eprintln!("lines of the recording that are not frames: {}", totals.not_frames);
    }
    eprintln!("time taken: {took_s:.1} s");
    if speed > 0.0 {
        eprintln!("how late the frames left: {}", totals.late.describe());
    }

    if let Err(error) = result {
        let reason = match error.kind() {
            ErrorKind::BrokenPipe | ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted => {
                "the other end closed the connection".to_string()
            }
            _ => error.to_string(),
        };
        return Err(format!("stopped before the end of the recording: {reason}").into());
    }
    Ok(())
}

/// Connects to the gateway. If nothing is listening there yet, it tries again every second.
fn connect(address: &str) -> io::Result<TcpStream> {
    let mut waiting = false;
    loop {
        match TcpStream::connect(address) {
            Ok(stream) => {
                // Without this the system may hold small packets back to send them together.
                stream.set_nodelay(true)?;
                eprintln!("connected to the gateway at {address}");
                return Ok(stream);
            }
            Err(error) if error.kind() == ErrorKind::ConnectionRefused => {
                if !waiting {
                    eprintln!("waiting for the gateway at {address}: start signal_receiver there");
                    waiting = true;
                }
                thread::sleep(Duration::from_secs(1));
            }
            Err(error) => return Err(with_name(address, error)),
        }
    }
}

/// Reads the recording frame by frame and writes one line per decoded message, each at its moment.
fn stream(
    log: impl BufRead,
    dbc: &Dbc,
    out: &mut dyn Write,
    speed: f64,
    limit: Option<f64>,
    totals: &mut Totals,
) -> io::Result<()> {
    // Time of the first frame in the recording, and the clock reading when it was sent.
    let mut start: Option<(f64, Instant)> = None;
    let mut line_out = String::with_capacity(2048);
    let mut next_report = REPORT_EVERY_S;
    // Worst lateness and messages sent, both since the last progress line.
    let (mut late_s, mut reported) = (0.0f64, 0u64);

    for line in log.lines() {
        let line = line?;
        let Some(frame) = parse_line(&line) else {
            if !line.trim().is_empty() {
                totals.not_frames += 1;
            }
            continue;
        };
        let (t0, clock) = *start.get_or_insert_with(|| (frame.t, Instant::now()));
        let t = frame.t - t0;
        if limit.is_some_and(|limit| t >= limit) {
            break;
        }
        totals.frames += 1;
        totals.recording_s = t;

        // Real time: a frame must not leave before its moment in the recording.
        if speed > 0.0 {
            let due = t / speed;
            let wait = due - clock.elapsed().as_secs_f64();
            if wait > 0.0 {
                out.flush()?; // what is already written leaves now, not after the wait
                thread::sleep(Duration::from_secs_f64(wait));
            }
            let late = clock.elapsed().as_secs_f64() - due;
            totals.late.add(late);
            late_s = late_s.max(late);
        }

        match dbc.by_id(frame.id) {
            None => *totals.unknown_ids.entry(frame.id).or_insert(0) += 1,
            Some(message) => {
                let decoded = message
                    .signals
                    .iter()
                    .enumerate()
                    .filter_map(|(i, signal)| message.value(i, &frame.data).map(|value| (signal.name.as_str(), value)));
                line_out.clear();
                let n = wire::encode(&mut line_out, t, frame.id, &message.name, decoded);
                line_out.push('\n');
                out.write_all(line_out.as_bytes())?;
                totals.messages += 1;
                totals.values += n as u64;
                totals.sent_ids.insert(frame.id);
            }
        }

        if t >= next_report {
            eprintln!(
                "t={t:7.1} s | {:9} messages | {:5.0} per second | at most {:.2} ms late",
                totals.messages,
                (totals.messages - reported) as f64 / REPORT_EVERY_S,
                1000.0 * late_s
            );
            (late_s, reported) = (0.0, totals.messages);
            while next_report <= t {
                next_report += REPORT_EVERY_S;
            }
        }
    }
    out.flush()
}
