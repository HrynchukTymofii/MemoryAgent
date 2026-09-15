//! Summarising a meeting transcript.
//!
//! Not on the capture path, so none of the router's latency rules apply: this
//! runs once when a recording stops, reads the whole transcript, and may take a
//! minute. What leaves the machine is the transcript and nothing else.

use std::time::Duration;

use serde_json::{json, Value};

use crate::{Cloud, CloudError};

/// A two-hour call is a long read and a long write. Past this something is
/// wrong, and the user can press Summarize again.
const TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Room for the thinking and a summary of a long meeting, and still inside a
/// single request that does not need streaming.
const MAX_TOKENS: u32 = 16_000;

const SYSTEM: &str = "\
You summarise meeting transcripts for the person who recorded them.

The transcript came from speech recognition, so expect misheard words and \
broken sentences; read through them for what was meant. \"Me\" is the person \
who recorded it. \"Them\" is everyone else on the call, mixed into one voice, \
so work out who is speaking from what they say. Lines of \"Me\" can include \
fragments of the other side picked up by the microphone; ignore anything that \
only repeats \"Them\".

Write Markdown, in the language of the transcript, with these sections in this \
order, each as a `###` heading, leaving out any that would be empty:

### Summary
Two to four sentences: what the meeting was, and how it went.

### Key points
What was said that matters later, as bullets.

### Questions asked
Only for an interview or a call made of questions: each question as a bullet, \
with one line on how it was answered.

### Decisions

### Action items
Bullets, each saying who does it when the transcript says.

Write only these sections: no title, no preamble, and no closing remarks. \
Never invent names, numbers, dates or commitments that are not in the transcript.";

impl Cloud {
    /// Summarise a meeting transcript as Markdown sections.
    pub fn summarize(&self, transcript: &str) -> Result<String, CloudError> {
        let body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "thinking": {"type": "adaptive"},
            // A declined request is retried on another model server-side rather
            // than ending in no summary.
            "fallbacks": "default",
            "system": SYSTEM,
            "messages": [{"role": "user", "content": format!("<transcript>\n{transcript}\n</transcript>")}],
        });
        text_of(&self.send(&body, TIMEOUT)?)
    }
}

/// The summary out of a reply, or why there is none.
fn text_of(reply: &Value) -> Result<String, CloudError> {
    match reply.get("stop_reason").and_then(Value::as_str) {
        Some("refusal") => {
            let why = reply
                .get("stop_details")
                .and_then(|d| d.get("explanation"))
                .and_then(Value::as_str)
                .unwrap_or("no reason given");
            return Err(CloudError::Refused(why.to_string()));
        }
        // Half a summary reads like a whole one. Better to say it did not fit.
        Some("max_tokens") => {
            return Err(CloudError::Malformed(
                "the summary ran past its length limit".into(),
            ))
        }
        _ => {}
    }
    let text = reply
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| CloudError::Malformed("no content".into()))?
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    let text = text.trim();
    if text.is_empty() {
        return Err(CloudError::Malformed("an empty summary".into()));
    }
    Ok(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinking_is_not_part_of_the_summary() {
        let reply = json!({
            "stop_reason": "end_turn",
            "content": [
                {"type": "thinking", "thinking": ""},
                {"type": "text", "text": "### Summary\nAn interview."}
            ]
        });
        assert_eq!(text_of(&reply).unwrap(), "### Summary\nAn interview.");
    }

    #[test]
    fn a_refusal_or_a_cut_off_summary_is_an_error() {
        assert!(matches!(
            text_of(&json!({"stop_reason": "refusal", "content": []})),
            Err(CloudError::Refused(_))
        ));
        assert!(text_of(
            &json!({"stop_reason": "max_tokens", "content": [{"type": "text", "text": "### Sum"}]})
        )
        .is_err());
    }
}
