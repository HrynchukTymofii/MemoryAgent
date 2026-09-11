//! The action space, as the model is allowed to see it.
//!
//! One tool per [`Intent`] the agent can execute, and the conversion back from
//! a tool call to the `(intent, slots)` pair the rest of the system already
//! speaks. Nothing here reaches the store: a tool call is a *request* for an
//! action, and `memos-agent` is still the only thing that performs one.
//!
//! ## Why this list is the design
//!
//! ADR-0003 gave the router seven intents and a grammar that could only name
//! collections that already existed, so "file this under something new" was not
//! a command the system could express — the model was not wrong about it, it
//! was mute. The lesson is that the action space, not the model, is the ceiling.
//! Adding a capability means adding a tool here; everything else follows.

use memos_core::{Intent, Slots};
use serde_json::{json, Value};

/// Longest free text accepted from a tool call, in characters.
///
/// The store truncates for its own reasons; this is about not writing a
/// model's runaway sentence into a title field in the first place.
const MAX_TEXT: usize = 200;

/// One step the model asked for, already in the shape `memos-agent` executes.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// The `tool_use` id, which the result has to be addressed back to.
    pub id: String,
    pub intent: Intent,
    pub slots: Slots,
}

/// Every tool, as JSON schema for the request body.
///
/// `strict` is set on all of them: the arguments then validate against the
/// schema exactly, which is the cloud equivalent of what the GBNF grammar did
/// for the local router — with one deliberate difference. The old grammar
/// enumerated the user's collections as literal alternatives, so a destination
/// that did not exist could not be *said*. Here a collection is a plain string
/// and an unknown one comes back as "no collection called X", because the model
/// can now do something about that: make it, then file into it.
pub fn schemas() -> Value {
    json!([
        tool(
            "save",
            "Save what the user is looking at right now — the selected text, or \
             the page — into memory. This is the default for \"save this\", \
             \"remember this\", \"add this to X\". Use the collection argument \
             only with a path from the collection list; if the user named a \
             place that is not on the list, call create_collection first.",
            json!({
                "collection": {"type": ["string", "null"], "description": "Full path of an existing collection, e.g. \"Study/Programming/React\". Null files it unsorted."},
                "title": {"type": ["string", "null"], "description": "A short title, if the user dictated one. Null takes the title from the page."},
                "tags": {"type": "array", "items": {"type": "string"}, "description": "Tags the user asked for. Usually empty."}
            }),
            &["collection", "title", "tags"],
        ),
        tool(
            "create_collection",
            "Make a new collection. Use it when the user asks for one, or when \
             they want something filed somewhere that is not on the list yet — \
             then call save with the new path.",
            json!({
                "name": {"type": "string", "description": "The new collection's own name, without any parent path."},
                "parent": {"type": ["string", "null"], "description": "Full path of an existing collection to put it under, or null for a new top-level one."}
            }),
            &["name", "parent"],
        ),
        tool(
            "note",
            "Write down something the user said, with no page or selection \
             behind it — a thought, a fact, a reminder to themselves.",
            json!({
                "text": {"type": "string", "description": "What to write down, in the user's own words."},
                "collection": {"type": ["string", "null"], "description": "Full path of an existing collection, or null."}
            }),
            &["text", "collection"],
        ),
        tool(
            "search",
            "Find things already in memory and show them to the user.",
            json!({"query": {"type": "string", "description": "What to look for, by meaning or by words."}}),
            &["query"],
        ),
        tool(
            "open",
            "Reopen the original source of a memory — the page or file it came from.",
            json!({"query": {"type": "string", "description": "Which memory to reopen."}}),
            &["query"],
        ),
        tool(
            "move_last",
            "Refile the most recently saved memory into a different collection. \
             This is for \"no, put that in X\" right after a save.",
            json!({"collection": {"type": "string", "description": "Full path of an existing collection."}}),
            &["collection"],
        ),
        tool(
            "tag_last",
            "Attach a tag to the most recently saved memory.",
            json!({"tag": {"type": "string", "description": "One tag, without a leading #."}}),
            &["tag"],
        ),
        tool(
            "task",
            "Add something to the user's to-do list.",
            json!({"title": {"type": "string", "description": "What they have to do."}}),
            &["title"],
        ),
        tool(
            "undo",
            "Take back the last thing this app did. Only when the user asks.",
            json!({}),
            &[],
        ),
    ])
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "strict": true,
        "input_schema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        },
    })
}

/// Turn one `tool_use` block into a step, or say why it is not one.
///
/// An unknown tool name is possible in principle and meaningless in practice —
/// the model is only offered the list above — but it is reported rather than
/// ignored, because a silently dropped step would leave the model waiting for a
/// result to a call that never ran.
pub fn step(id: &str, name: &str, input: &Value) -> Result<Step, String> {
    let text = |key: &str| -> Option<String> {
        input
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.chars().take(MAX_TEXT).collect())
    };

    let (intent, slots) = match name {
        "save" => (
            Intent::Save,
            Slots {
                collection: text("collection"),
                title: text("title"),
                tags: input
                    .get("tags")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::trim)
                            .filter(|t| !t.is_empty())
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default(),
                ..Default::default()
            },
        ),
        // `title` carries the new name and `collection` the parent, which is
        // the convention every other intent already follows: `collection` is
        // always a path that exists, never one being made.
        "create_collection" => (
            Intent::CreateCollection,
            Slots {
                title: text("name").ok_or_else(|| "create_collection needs a name".to_string())?.into(),
                collection: text("parent"),
                ..Default::default()
            },
        ),
        "note" => (
            Intent::Note,
            Slots {
                title: text("text"),
                collection: text("collection"),
                ..Default::default()
            },
        ),
        "search" => (
            Intent::Search,
            Slots { query: text("query"), ..Default::default() },
        ),
        "open" => (
            Intent::Open,
            Slots { query: text("query"), ..Default::default() },
        ),
        "move_last" => (
            Intent::Move,
            Slots { collection: text("collection"), ..Default::default() },
        ),
        "tag_last" => (
            Intent::Tag,
            Slots {
                tags: text("tag").into_iter().collect(),
                ..Default::default()
            },
        ),
        "task" => (
            Intent::Task,
            Slots { title: text("title"), ..Default::default() },
        ),
        "undo" => (Intent::Undo, Slots::default()),
        other => return Err(format!("no tool called {other}")),
    };

    Ok(Step { id: id.to_string(), intent, slots })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one the whole rewrite exists for.
    #[test]
    fn a_new_collection_is_a_name_and_a_parent() {
        let s = step(
            "toolu_1",
            "create_collection",
            &json!({"name": "Public Speaking", "parent": "Study"}),
        )
        .unwrap();
        assert_eq!(s.intent, Intent::CreateCollection);
        assert_eq!(s.slots.title.as_deref(), Some("Public Speaking"));
        assert_eq!(s.slots.collection.as_deref(), Some("Study"));
    }

    /// JSON null is how the schema says "not given", and it has to arrive as
    /// `None` rather than as the string "null" or an empty destination that
    /// would file a memory into a collection called nothing.
    #[test]
    fn a_null_argument_is_an_absent_one() {
        let s = step("t", "save", &json!({"collection": null, "title": null, "tags": []})).unwrap();
        assert_eq!(s.intent, Intent::Save);
        assert!(s.slots.collection.is_none());
        assert!(s.slots.title.is_none());
        assert!(s.slots.tags.is_empty());
    }

    #[test]
    fn whitespace_is_not_a_slot() {
        let s = step("t", "save", &json!({"collection": "   ", "tags": ["", " react "]})).unwrap();
        assert!(s.slots.collection.is_none());
        assert_eq!(s.slots.tags, vec!["react"]);
    }

    #[test]
    fn an_unknown_tool_is_reported_not_dropped() {
        assert!(step("t", "delete_everything", &json!({})).is_err());
    }

    /// Every tool the prompt offers has to be one `step` can convert, or the
    /// model gets told "no tool called X" about a tool we advertised.
    #[test]
    fn every_advertised_tool_converts() {
        for t in schemas().as_array().unwrap() {
            let name = t["name"].as_str().unwrap();
            let mut input = json!({});
            // Fill every declared property with something of the right type,
            // so this exercises the conversion rather than the empty case.
            for (key, schema) in t["input_schema"]["properties"].as_object().unwrap() {
                let ty = &schema["type"];
                let array = ty == "array";
                input[key] = if array { json!(["x"]) } else { json!("x") };
            }
            assert!(step("t", name, &input).is_ok(), "{name} is advertised but not convertible");
        }
    }
}
