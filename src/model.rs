//! Asking a language model about one window of the knowledge graph.
//!
//! The model runs in another process, `llama-server` of llama.cpp, and is reached over HTTP on
//! this same machine. This module writes the requests, reads the replies and keeps what each
//! answer cost the model.
//!
//! A question is asked in three steps. Once, the server is asked how this model wants a
//! conversation written, with the prefix of the question as the system message and a mark where
//! the window will go (`frame_request`). Once, the part before the mark is sent alone, so that
//! the model reads it and the server keeps that state (`prefix_request`). Then each window is
//! sent after that same part (`request`): the model starts from the kept state and reads only
//! the window.
//!
//! Nothing here is a general HTTP or JSON library: it writes exactly the requests `llama-server`
//! takes and reads the few fields of its replies that are kept.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// The most tokens an answer may have. An option is one or two words.
const MAX_TOKENS: u32 = 12;

/// Stands where the window will go when the server is asked how to write the conversation.
const MARK: &str = "@@WINDOW@@";

/// The fields of the reply that are kept. Asked for by name, the server sends only these.
const FIELDS: &str = "[\"content\",\"tokens_evaluated\",\"timings/cache_n\",\"timings/prompt_n\",\
                      \"timings/predicted_n\",\"timings/prompt_ms\",\"timings/predicted_ms\"]";

/// How long to wait for the server before giving up.
const PATIENCE: Duration = Duration::from_secs(300);

/// The conversation of one question as this model wants it written: what goes before the
/// window, which holds the prefix, and what goes after it.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Frame {
    pub head: String,
    pub tail: String,
}

/// What the model answered about one window and what it took to answer.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Reply {
    pub answer: String,
    /// Tokens of the whole prompt: the prefix of the question and the window.
    pub prompt_tokens: u32,
    /// Of those, the tokens the server had kept and the model did not read again.
    pub cached_tokens: u32,
    /// The tokens it had to read this time.
    pub evaluated_tokens: u32,
    /// The tokens it wrote.
    pub generated_tokens: u32,
    /// Milliseconds reading the prompt and milliseconds writing the answer.
    pub prompt_ms: f64,
    pub generation_ms: f64,
}

/// The options of a question, taken from the line of its prefix that lists them.
pub fn options_of(prefix: &str) -> Vec<String> {
    let list = prefix.lines().find_map(|line| line.strip_prefix("Options: ")).unwrap_or("");
    list.split(',').map(str::trim).filter(|option| !option.is_empty()).map(str::to_string).collect()
}

/// A text as JSON writes it: between quotes, with the characters JSON does not take as they are
/// written in its own way.
pub fn quoted(text: &str) -> String {
    let mut json = String::with_capacity(text.len() + 2);
    json.push('"');
    for c in text.chars() {
        match c {
            '"' => json.push_str("\\\""),
            '\\' => json.push_str("\\\\"),
            '\n' => json.push_str("\\n"),
            c if u32::from(c) < 0x20 => json.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => json.push(c),
        }
    }
    json.push('"');
    json
}

/// A grammar, as llama.cpp writes them, that lets the model write one of the options and
/// nothing else.
pub fn grammar(options: &[String]) -> String {
    let quoted: Vec<String> = options.iter().map(|option| quoted(option)).collect();
    format!("root ::= {}", quoted.join(" | "))
}

/// Asks the server how this model writes a conversation with the prefix as the system message
/// and the window as the message of the user. For `POST /apply-template`.
pub fn frame_request(prefix: &str) -> String {
    format!(
        "{{\"messages\":[{{\"role\":\"system\",\"content\":{}}},{{\"role\":\"user\",\"content\":\"{MARK}\"}}]}}",
        quoted(prefix)
    )
}

/// Reads the reply to `frame_request`: the conversation, cut where the window goes.
pub fn read_frame(body: &str) -> Result<Frame, String> {
    let prompt = text(body, "prompt").ok_or("the server did not write the conversation")?;
    let (head, tail) = prompt.split_once(MARK).ok_or("the conversation lost the mark of the window")?;
    Ok(Frame { head: head.to_string(), tail: tail.to_string() })
}

/// Has the model read what goes before the window, and nothing written. The server keeps the
/// state of the model at the end of it in `slot`, the place it holds for this question. For
/// `POST /completion`.
pub fn prefix_request(frame: &Frame, slot: usize) -> String {
    format!(
        "{{\"prompt\":{},\"n_predict\":0,\"cache_prompt\":true,\"id_slot\":{slot},\"response_fields\":{FIELDS}}}",
        quoted(&frame.head)
    )
}

/// The request for one window: the same start as `prefix_request`, then the window and the
/// end of the conversation, and the answer held to the options. For `POST /completion`.
pub fn request(frame: &Frame, window: &str, options: &[String], slot: usize) -> String {
    let prompt = quoted(&format!("{}{window}{}", frame.head, frame.tail));
    let mut body = format!(
        "{{\"prompt\":{prompt},\"n_predict\":{MAX_TOKENS},\"temperature\":0,\"cache_prompt\":true,\
         \"id_slot\":{slot},\"response_fields\":{FIELDS}"
    );
    if !options.is_empty() {
        body.push_str(&format!(",\"grammar\":{}", quoted(&grammar(options))));
    }
    body.push('}');
    body
}

/// What follows `"key":` the first time that key appears in a piece of JSON.
fn after<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let at = json.find(&format!("\"{key}\":"))?;
    Some(json[at + key.len() + 3..].trim_start())
}

/// The number under a key, the first time the key appears.
pub fn number(json: &str, key: &str) -> Option<f64> {
    let rest = after(json, key)?;
    let end = rest.find(|c: char| !(c.is_ascii_digit() || "+-.eE".contains(c))).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// The text under a key, the first time the key appears, with what JSON wrote in its own way
/// turned back into characters.
pub fn text(json: &str, key: &str) -> Option<String> {
    let mut chars = after(json, key)?.strip_prefix('"')?.chars();
    let mut text = String::new();
    loop {
        match chars.next()? {
            '"' => return Some(text),
            '\\' => match chars.next()? {
                'n' => text.push('\n'),
                'r' => text.push('\r'),
                't' => text.push('\t'),
                'u' => {
                    // Four hexadecimal digits. Half of a pair, which is no character alone,
                    // becomes the replacement character.
                    let digits: String = chars.by_ref().take(4).collect();
                    let code = u32::from_str_radix(&digits, 16).ok()?;
                    text.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                }
                other => text.push(other),
            },
            other => text.push(other),
        }
    }
}

/// Reads the reply of `llama-server` to `request` or to `prefix_request`.
pub fn read_reply(body: &str) -> Result<Reply, String> {
    let Some(answer) = text(body, "content") else {
        let start: String = body.chars().take(200).collect();
        return Err(format!("the reply of the model has no answer: {start}"));
    };
    // The server names these fields with the group they belong to; inside the whole reply they
    // have their short name.
    let count = |key: &str| number(body, &format!("timings/{key}")).or_else(|| number(body, key)).unwrap_or(0.0);
    Ok(Reply {
        answer: answer.trim().to_string(),
        prompt_tokens: number(body, "tokens_evaluated").unwrap_or(0.0) as u32,
        cached_tokens: count("cache_n") as u32,
        evaluated_tokens: count("prompt_n") as u32,
        generated_tokens: count("predicted_n") as u32,
        prompt_ms: count("prompt_ms"),
        generation_ms: count("predicted_ms"),
    })
}

/// Sends one request to the server and returns the body of its reply. Each request opens its
/// own connection and the server closes it when it has answered.
pub fn call(server: &str, method: &str, path: &str, body: &str) -> io::Result<String> {
    let mut stream = TcpStream::connect(server).map_err(|e| crate::with_name(server, e))?;
    stream.set_read_timeout(Some(PATIENCE))?;
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {server}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    let Some((head, body)) = reply.split_once("\r\n\r\n") else {
        return Err(io::Error::other(format!("{server}: the reply is not HTTP")));
    };
    let status = head.split_whitespace().nth(1).unwrap_or("");
    if status != "200" {
        return Err(io::Error::other(format!("{server}{path} answered {status}: {}", body.trim())));
    }
    Ok(body.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIX: &str = "Question: is the car braking hard?\nOptions: yes, no\nSignals:\n1. \"speed\" in km/h\n";

    #[test]
    fn the_options_and_the_grammar_come_from_the_prefix() {
        let options = options_of(PREFIX);
        assert_eq!(options, ["yes", "no"]);
        assert_eq!(grammar(&options), "root ::= \"yes\" | \"no\"");
        assert!(options_of("Question: no list here\n").is_empty());
    }

    #[test]
    fn text_goes_into_json_and_comes_back_the_same() {
        let said = "a \"quoted\" word, a back\\slash,\na tab\there and 5 m/s²";
        let json = format!("{{\"n\":3,\"said\":{},\"t\":-2.5e1}}", quoted(said));
        assert!(!json.contains('\n') && !json.contains('\t'));
        assert_eq!(text(&json, "said").as_deref(), Some(said));
        assert_eq!(number(&json, "t"), Some(-25.0));
        assert_eq!(number(&json, "n"), Some(3.0));
        assert_eq!(text(&json, "missing"), None);
        // As other programs may write the same text.
        assert_eq!(text("{\"said\": \"a\\/b \\u00e9\"}", "said").as_deref(), Some("a/b é"));
    }

    #[test]
    fn the_window_goes_between_the_two_parts_of_the_frame() {
        // What `POST /apply-template` of llama-server answered for LFM2.5, shortened.
        let reply = "{\"prompt\":\"<|startoftext|><|im_start|>system\\nQuestion?\\nOptions: yes, no\\n<|im_end|>\\n\
                     <|im_start|>user\\n@@WINDOW@@<|im_end|>\\n<|im_start|>assistant\\n\"}";
        let frame = read_frame(reply).unwrap();
        assert!(frame.head.ends_with("<|im_start|>user\n"));
        assert_eq!(frame.tail, "<|im_end|>\n<|im_start|>assistant\n");
        assert!(read_frame("{\"prompt\":\"no mark\"}").is_err());

        let first = prefix_request(&frame, 2);
        assert!(first.starts_with("{\"prompt\":\"<|startoftext|>"));
        assert!(first.contains("user\\n\",\"n_predict\":0,\"cache_prompt\":true,\"id_slot\":2,"));

        let options = options_of("Options: yes, no\n");
        let body = request(&frame, "Window of 5 s:\nAnswer:", &options, 2);
        // The request of a window starts exactly as the one that had the prefix read.
        let shared = first.find("\",\"n_predict\"").unwrap();
        assert_eq!(body[..shared], first[..shared]);
        assert!(body.contains("user\\nWindow of 5 s:\\nAnswer:<|im_end|>\\n<|im_start|>assistant\\n\",\"n_predict\":12,"));
        assert!(body.ends_with(",\"grammar\":\"root ::= \\\"yes\\\" | \\\"no\\\"\"}"));
        assert!(!request(&frame, "w", &[], 0).contains("grammar"));
    }

    #[test]
    fn the_reply_of_the_server_is_read() {
        // A reply of llama-server (commit de30697) to a request written by `request`.
        let body = "{\"content\":\"no\",\"tokens_evaluated\":306,\"timings/cache_n\":181,\"timings/prompt_n\":125,\
                    \"timings/predicted_n\":3,\"timings/prompt_ms\":27.244,\"timings/predicted_ms\":52.68}";
        let reply = read_reply(body).unwrap();
        assert_eq!(reply.answer, "no");
        assert_eq!((reply.prompt_tokens, reply.cached_tokens, reply.evaluated_tokens), (306, 181, 125));
        assert_eq!((reply.generated_tokens, reply.prompt_ms, reply.generation_ms), (3, 27.244, 52.68));
        // The same fields inside their group, as the whole reply has them.
        let whole = "{\"content\":\" yes\\n\",\"tokens_evaluated\":40,\"timings\":{\"cache_n\":30,\"prompt_n\":10,\
                     \"prompt_ms\":1.5,\"predicted_n\":2,\"predicted_ms\":3.0}}";
        let reply = read_reply(whole).unwrap();
        assert_eq!((reply.answer.as_str(), reply.cached_tokens, reply.evaluated_tokens), ("yes", 30, 10));
        assert!(read_reply("{\"error\":{\"code\":400}}").unwrap_err().contains("no answer"));
    }
}
