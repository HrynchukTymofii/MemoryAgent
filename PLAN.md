# A task no longer links itself to the newest memory

- crates/memos-agent/src/execute.rs — `task()` drops the `most_recent_capture()`
  lookup: no `item_id`, no "about …" provenance. The test that asserted the link
  becomes the test that asserts there is none.
