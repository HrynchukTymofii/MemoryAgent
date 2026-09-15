//! The loop, driven by a stub that answers with canned tool calls.
//!
//! What is under test is the conversation, not the model: whether a tool call
//! becomes a step, whether its result goes back addressed to the call it
//! answers, whether a second turn sees the first one's outcome, and whether the
//! turn ends when the model stops asking for things. Every one of those is a
//! way for this tier to be silently broken while looking configured — which is
//! the failure ADR-0011 exists to stop repeating — and none of them needs a
//! live model, or should be paid for on every test run.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc::channel;

use memos_context::Context;
use serde_json::{json, Value};

/// Read one request, whole. A server that answers before it has read the body
/// leaves the client writing into a closed socket, which surfaces as a network
/// error and looks nothing like the thing under test.
fn read_request(socket: &std::net::TcpStream) -> Value {
    let mut reader = BufReader::new(socket.try_clone().unwrap());
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = n.trim().parse().unwrap_or(0);
        }
        if line == "\r\n" {
            break;
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).expect("the whole body");
    serde_json::from_slice(&body).expect("JSON went out")
}

fn answer(socket: &mut std::net::TcpStream, status: &str, payload: &str) {
    let _ = socket.write_all(
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
            payload.len()
        )
        .as_bytes(),
    );
    let _ = socket.flush();
}

/// Answer `replies` in order, one per request, and hand every request body back
/// down the channel so the test can look at what was sent.
fn stub(replies: Vec<Value>) -> (String, std::sync::mpsc::Receiver<Value>) {
    serve(replies.into_iter().map(|r| ("200 OK".to_string(), r.to_string())).collect())
}

fn serve(replies: Vec<(String, String)>) -> (String, std::sync::mpsc::Receiver<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = channel();

    std::thread::spawn(move || {
        for (status, payload) in replies {
            let (mut socket, _) = listener.accept().expect("a connection");
            let _ = tx.send(read_request(&socket));
            answer(&mut socket, &status, &payload);
        }
    });

    (base, rx)
}

fn tool_use(id: &str, name: &str, input: Value) -> Value {
    json!({"type": "tool_use", "id": id, "name": name, "input": input})
}

fn reply(content: Value, stop: &str) -> Value {
    json!({"id": "msg_1", "type": "message", "role": "assistant", "content": content, "stop_reason": stop})
}

/// The command that started all of this: a page, a collection that does not
/// exist, and one sentence asking for both.
#[test]
fn a_command_can_make_a_collection_and_then_file_into_it() {
    let (base, sent) = stub(vec![
        reply(
            json!([tool_use(
                "toolu_1",
                "create_collection",
                json!({"name": "Public Speaking", "parent": "Study"})
            )]),
            "tool_use",
        ),
        reply(
            json!([tool_use(
                "toolu_2",
                "save",
                json!({"collection": "Study/Public Speaking", "title": null, "tags": []})
            )]),
            "tool_use",
        ),
        reply(json!([{"type": "text", "text": "Saved under Study / Public Speaking."}]), "end_turn"),
    ]);
    let cloud = memos_cloud::Cloud::new(Some("sk-ant-test".into()))
        .unwrap()
        .with_base(&base);
    let ctx = Context {
        current_url: Some("https://unprompted.cool".into()),
        ..Default::default()
    };

    let mut receipts = Vec::new();
    let plan = cloud
        .run(
            "add this website to the memory about public speaking, create a new collection",
            &ctx,
            &["Study".to_string()],
            |step| {
                receipts.push(step.intent.as_str());
                match step.intent {
                    memos_core::Intent::CreateCollection => "New collection: Study / Public Speaking".into(),
                    _ => "Saved to Study / Public Speaking".into(),
                }
            },
        )
        .expect("the plan ran");

    assert_eq!(receipts, vec!["CREATE_COLLECTION", "SAVE"], "both halves, in order");
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(plan.steps[1].slots.collection.as_deref(), Some("Study/Public Speaking"));
    assert_eq!(plan.say.as_deref(), Some("Saved under Study / Public Speaking."));

    // What the model was told, turn by turn. The second request has to carry
    // the first call and its result, or the model is deciding the second step
    // without knowing the first one happened.
    let first = sent.recv().unwrap();
    assert_eq!(first["messages"].as_array().unwrap().len(), 1);
    assert_eq!(first["model"], memos_cloud::MODEL);
    assert!(first["tools"].as_array().unwrap().len() >= 9, "the action space went out");

    let second = sent.recv().unwrap();
    let messages = second["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3, "user, assistant, tool results");
    assert_eq!(messages[1]["role"], "assistant");
    let result = &messages[2]["content"][0];
    assert_eq!(result["type"], "tool_result");
    assert_eq!(result["tool_use_id"], "toolu_1", "addressed to the call it answers");
    assert_eq!(result["content"], "New collection: Study / Public Speaking");
}

/// Two calls in one turn are one user message of results. Splitting them is how
/// a harness quietly teaches the model to stop asking for more than one thing.
#[test]
fn results_from_one_turn_go_back_together() {
    let (base, sent) = stub(vec![
        reply(
            json!([
                tool_use("a", "task", json!({"title": "send the CV"})),
                tool_use("b", "note", json!({"text": "they use Rust", "collection": null})),
            ]),
            "tool_use",
        ),
        reply(json!([{"type": "text", "text": "Done."}]), "end_turn"),
    ]);
    let cloud = memos_cloud::Cloud::new(Some("sk-ant-test".into()))
        .unwrap()
        .with_base(&base);
    let plan = cloud
        .run("add a task and note this", &Context::default(), &[], |_| "ok".into())
        .unwrap();
    assert_eq!(plan.steps.len(), 2);

    let _ = sent.recv().unwrap();
    let second = sent.recv().unwrap();
    let messages = second["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3, "one user message, not two");
    assert_eq!(messages[2]["content"].as_array().unwrap().len(), 2);
}

/// A policy decline is not an outcome to show the user — it is the signal to
/// fall back to the grammar, so it has to arrive as an error.
#[test]
fn a_refusal_is_an_error_the_caller_can_fall_back_from() {
    let (base, _sent) = stub(vec![json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "content": [],
        "stop_reason": "refusal",
        "stop_details": {"type": "refusal", "category": "cyber", "explanation": "declined"},
    })]);
    let cloud = memos_cloud::Cloud::new(Some("sk-ant-test".into()))
        .unwrap()
        .with_base(&base);
    let err = cloud
        .run("do something", &Context::default(), &[], |_| "ok".into())
        .expect_err("a refusal is not a plan");
    assert!(matches!(err, memos_cloud::CloudError::Refused(_)), "{err}");
}

/// An HTTP error carries the reason in its body — a bad key, a rate limit — and
/// losing it would send whoever reads the log looking in the wrong place.
#[test]
fn a_rejected_request_says_why() {
    let body = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
    let (base, _sent) = serve(vec![("401 Unauthorized".into(), body.into())]);

    let cloud = memos_cloud::Cloud::new(Some("sk-ant-bad".into()))
        .unwrap()
        .with_base(&base);
    let err = cloud
        .run("save this", &Context::default(), &[], |_| "ok".into())
        .expect_err("401 is not a plan");
    assert!(err.to_string().contains("invalid x-api-key"), "{err}");
}

/// The summary request carries the transcript and nothing that belongs to the
/// router: no tools, no collections.
#[test]
fn a_summary_sends_the_transcript_and_returns_the_text() {
    let (base, sent) = stub(vec![reply(
        json!([{"type": "text", "text": "### Summary\nA short interview."}]),
        "end_turn",
    )]);
    let cloud = memos_cloud::Cloud::new(Some("sk-ant-test".into()))
        .unwrap()
        .with_base(&base);

    let summary = cloud
        .summarize("**00:05 Them:** Tell me about yourself.")
        .expect("a summary");
    assert_eq!(summary, "### Summary\nA short interview.");

    let body = sent.recv().unwrap();
    assert_eq!(body["model"], "claude-opus-5");
    assert!(body.get("tools").is_none());
    assert!(body["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("Tell me about yourself."));
}
