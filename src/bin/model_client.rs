//! Asks a language model every window of the knowledge graph and records what each answer cost.
//!
//! `signal_receiver --prompts FILE` writes what a model would read: the prefix of each question
//! once, and then one line per window. This program reads that file. It has the model read the
//! prefix of each question once, sends each window after it, and writes one line per answer with
//! the tokens, the time and the CPU and memory of the model's process.

use std::error::Error;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use vehicle_smart_telemetry::metrics::{cpu_seconds, cpu_seconds_of, memory_mb, memory_mb_of, process_named};
use vehicle_smart_telemetry::model::{self, Frame};
use vehicle_smart_telemetry::options::Args;
use vehicle_smart_telemetry::with_name;

const HELP: &str = "\
model_client: asks a language model every window of the knowledge graph and records the cost.

use:  model_client --prompts sample/prompts.ndjson
      model_client --prompts prompts.ndjson --follow --answers answers.ndjson

  --prompts FILE    what signal_receiver --prompts wrote (default prompts.ndjson)
  --answers FILE    where to write one line per answer (default answers.ndjson)
  --server ADDR     where llama-server listens (default 127.0.0.1:8080)
  --process NAME    process whose CPU and memory are measured (default llama-server)
  --follow          keep reading while the file grows, for a receiver that is still running
  --idle S          with --follow, seconds without a new line after which it stops (10)
  --quiet           do not print each answer";
const WITH_VALUE: [&str; 5] = ["--prompts", "--answers", "--server", "--process", "--idle"];
const WITHOUT_VALUE: [&str; 2] = ["--follow", "--quiet"];

/// One question: what goes around each of its windows and what its answers have been.
#[derive(Default)]
struct Asked {
    id: String,
    frame: Frame,
    options: Vec<String>,
    /// The slot of the server that keeps what the model has read of this question.
    slot: usize,
    /// What reading the prefix cost, once.
    prefix_tokens: u32,
    prefix_ms: f64,
    /// Each answer given and how many times.
    given: Vec<(String, u32)>,
    evaluated_tokens: u64,
    total_ms: f64,
}

/// What all the answers together have cost.
#[derive(Default)]
struct Totals {
    answers: u32,
    prompt_tokens: u64,
    cached_tokens: u64,
    evaluated_tokens: u64,
    generated_tokens: u64,
    prompt_ms: f64,
    generation_ms: f64,
    total_ms: f64,
    slowest_ms: f64,
    cpu_s: f64,
    /// Reading the prefixes, once each.
    prefix_tokens: u64,
    prefix_ms: f64,
}

struct Client {
    server: String,
    /// The process of the model, if it runs on this machine.
    pid: Option<String>,
    slots: usize,
    asked: Vec<Asked>,
    totals: Totals,
    answers: BufWriter<File>,
    show: bool,
}

impl Client {
    /// A new question: the model reads its prefix now, once, and the server keeps that state.
    fn register(&mut self, id: String, prefix: &str) -> Result<(), Box<dyn Error>> {
        // Each question takes the next slot of the server. Two questions in one slot would each
        // overwrite what the server keeps for the other.
        let slot = self.asked.len() % self.slots;
        if self.asked.len() >= self.slots {
            println!("'{id}' shares slot {slot}: its prefix will be read again at each window (see --parallel of the server)");
        }
        let layout = model::call(&self.server, "POST", "/apply-template", &model::frame_request(prefix))?;
        let frame = model::read_frame(&layout)?;
        let read = model::call(&self.server, "POST", "/completion", &model::prefix_request(&frame, slot))?;
        let read = model::read_reply(&read)?;
        if self.show {
            println!("question '{id}': slot {slot}, prefix of {} tokens read in {:.1} ms", read.prompt_tokens, read.prompt_ms);
        }
        self.totals.prefix_tokens += u64::from(read.prompt_tokens);
        self.totals.prefix_ms += read.prompt_ms;
        let (prefix_tokens, prefix_ms) = (read.prompt_tokens, read.prompt_ms);
        self.asked.push(Asked { id, frame, options: model::options_of(prefix), slot, prefix_tokens, prefix_ms, ..Asked::default() });
        Ok(())
    }

    /// Takes one line of the prompts: the prefix of a question, or one of its windows.
    fn take(&mut self, line: &str) -> Result<(), Box<dyn Error>> {
        let id = model::text(line, "question").ok_or("a line of the prompts names no question")?;
        if let Some(prefix) = model::text(line, "prefix") {
            return self.register(id, &prefix);
        }
        let (Some(window), Some(t)) = (model::text(line, "window"), model::number(line, "t")) else {
            return Err(format!("a line of '{id}' is neither its prefix nor a window").into());
        };
        let Some(asked) = self.asked.iter_mut().find(|asked| asked.id == id) else {
            return Err(format!("a window of '{id}' came before its prefix").into());
        };

        // The question: how long it takes here, and what the model's process spends meanwhile.
        let body = model::request(&asked.frame, &window, &asked.options, asked.slot);
        let cpu_before = self.pid.as_deref().map_or(0.0, cpu_seconds_of);
        let clock = Instant::now();
        let reply = model::read_reply(&model::call(&self.server, "POST", "/completion", &body)?)?;
        let total_ms = 1000.0 * clock.elapsed().as_secs_f64();
        let cpu_s = self.pid.as_deref().map_or(0.0, cpu_seconds_of) - cpu_before;
        let memory = self.pid.as_deref().map_or(0.0, |pid| memory_mb_of(pid).0);

        writeln!(
            self.answers,
            "{{\"question\":{},\"t\":{t},\"answer\":{},\"prompt_tokens\":{},\"cached_tokens\":{},\
             \"evaluated_tokens\":{},\"generated_tokens\":{},\"prompt_ms\":{:.2},\"generation_ms\":{:.2},\
             \"total_ms\":{total_ms:.2},\"cpu_ms\":{:.0},\"memory_mb\":{memory:.1}}}",
            model::quoted(&id),
            model::quoted(&reply.answer),
            reply.prompt_tokens,
            reply.cached_tokens,
            reply.evaluated_tokens,
            reply.generated_tokens,
            reply.prompt_ms,
            reply.generation_ms,
            1000.0 * cpu_s
        )?;
        self.answers.flush()?;
        if self.show {
            println!(
                "t={t:5} s | {id:<14} | {:<10} | {:4} tokens: {:4} kept, {:3} read, {:2} written | {total_ms:7.1} ms | {memory:6.1} MB",
                reply.answer, reply.prompt_tokens, reply.cached_tokens, reply.evaluated_tokens, reply.generated_tokens
            );
        }

        match asked.given.iter_mut().find(|(answer, _)| *answer == reply.answer) {
            Some((_, times)) => *times += 1,
            None => asked.given.push((reply.answer.clone(), 1)),
        }
        asked.evaluated_tokens += u64::from(reply.evaluated_tokens);
        asked.total_ms += total_ms;
        let totals = &mut self.totals;
        totals.answers += 1;
        totals.prompt_tokens += u64::from(reply.prompt_tokens);
        totals.cached_tokens += u64::from(reply.cached_tokens);
        totals.evaluated_tokens += u64::from(reply.evaluated_tokens);
        totals.generated_tokens += u64::from(reply.generated_tokens);
        totals.prompt_ms += reply.prompt_ms;
        totals.generation_ms += reply.generation_ms;
        totals.total_ms += total_ms;
        totals.slowest_ms = totals.slowest_ms.max(total_ms);
        totals.cpu_s += cpu_s;
        Ok(())
    }

    fn summary(&self) {
        let totals = &self.totals;
        let n = f64::from(totals.answers.max(1));
        println!("\n--- model_client summary ---");
        println!("answers: {} to {} questions", totals.answers, self.asked.len());
        println!("prefixes: {} tokens, read once in {:.1} ms", totals.prefix_tokens, totals.prefix_ms);
        println!(
            "windows: {} tokens in the prompts, {} of them kept by the server and {} read; {} written",
            totals.prompt_tokens, totals.cached_tokens, totals.evaluated_tokens, totals.generated_tokens
        );
        println!(
            "time per answer: {:.1} ms on average, {:.1} ms the slowest; inside the model, {:.1} ms reading and {:.1} ms writing",
            totals.total_ms / n,
            totals.slowest_ms,
            totals.prompt_ms / n,
            totals.generation_ms / n
        );
        match self.pid.as_deref() {
            Some(pid) => {
                let (now, peak) = memory_mb_of(pid);
                println!(
                    "model process {pid}: {:.2} s of CPU for these answers, {:.0} ms each; memory {now:.1} MB now, {peak:.1} MB at its peak",
                    totals.cpu_s,
                    1000.0 * totals.cpu_s / n
                );
            }
            None => println!("model process: not on this machine, so its CPU and memory were not measured"),
        }
        for asked in &self.asked {
            let times: u32 = asked.given.iter().map(|(_, times)| times).sum();
            let given: Vec<String> = asked.given.iter().map(|(answer, times)| format!("{answer} {times}")).collect();
            let each = f64::from(times.max(1));
            println!(
                "question '{}': prefix of {} tokens read in {:.1} ms; {times} answers ({}); {:.0} tokens read and {:.1} ms per answer",
                asked.id,
                asked.prefix_tokens,
                asked.prefix_ms,
                given.join(", "),
                asked.evaluated_tokens as f64 / each,
                asked.total_ms / each
            );
        }
        println!("cost of this process: {:.2} s of CPU, peak memory {:.1} MB", cpu_seconds(), memory_mb().1);
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let a = Args::parse(HELP, &WITH_VALUE, &WITHOUT_VALUE);
    let path = a.text("--prompts", "prompts.ndjson");
    let server = a.text("--server", "127.0.0.1:8080");
    let (follow, idle_s) = (a.enabled("--follow"), a.number("--idle", 10.0));

    // Before anything is asked, the server says which model it holds, how many slots it has and
    // how many tokens fit in each.
    let props = model::call(&server, "GET", "/props", "")?;
    let slots = model::number(&props, "total_slots").unwrap_or(1.0).max(1.0) as usize;
    let room = model::number(&props, "n_ctx").unwrap_or(0.0);
    let loaded = model::text(&props, "model_path").unwrap_or_default();
    let pid = process_named(&a.text("--process", "llama-server"));
    println!("model_client: {server} holds {loaded}, with {slots} slots of {room} tokens");
    if pid.is_none() {
        println!("no such process on this machine: the CPU and the memory of the model will not be measured");
    }

    let open = || File::open(&path).map(BufReader::new).map_err(|e| with_name(&path, e));
    let mut reader = open()?;

    let answers = a.text("--answers", "answers.ndjson");
    let mut client = Client {
        server,
        pid,
        slots,
        asked: Vec::new(),
        totals: Totals::default(),
        answers: BufWriter::new(File::create(&answers).map_err(|e| with_name(&answers, e))?),
        show: !a.enabled("--quiet"),
    };
    let (mut line, mut read, mut last_news) = (String::new(), 0u64, Instant::now());
    loop {
        let n = reader.read_line(&mut line)?;
        read += n as u64;
        if line.ends_with('\n') {
            client.take(&line)?;
            line.clear();
            last_news = Instant::now();
        } else if n == 0 {
            // The end of the file, for now or for good.
            if !follow || last_news.elapsed().as_secs_f64() >= idle_s {
                break;
            }
            // A receiver that starts again writes the file from its beginning.
            if fs::metadata(&path).map_or(0, |file| file.len()) < read {
                (reader, read) = (open()?, 0);
                line.clear();
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
    client.summary();
    println!("answers written to {answers}");
    Ok(())
}
