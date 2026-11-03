use nocturne::{changes, graph, store::Store};
use serde_json::Value;

fn create(s: &Store, body: &str) -> Value {
    s.write("a", None, |c| {
        graph::create(c, "a", "core://", "one", body, 0, None)
    })
    .unwrap()
}
fn edit(s: &Store, body: &str, batch: Option<&str>) -> Value {
    s.write("a", batch, |c| {
        graph::update(c, "a", "core://one", body, None)
    })
    .unwrap()
}

// Review must compare batch endpoints and suppress both intermediate text and net-zero content changes.
#[test]
fn batch_compares_start_to_finish() {
    let s = Store::memory().unwrap();
    create(&s, "start\n");
    edit(&s, "intermediate secret\n", Some("edits"));
    edit(&s, "finish\n", Some("edits"));
    let text = changes::diff_text(s.connection(), "a", "edits", true).unwrap();
    assert!(text.contains("-start\n+finish\n"), "{text}");
    assert!(!text.contains("intermediate secret"));
    edit(&s, "start\n", Some("edits"));
    let text = changes::diff_text(s.connection(), "a", "edits", true).unwrap();
    assert!(!text.contains("@@"));
    assert!(text.contains("versions:"));
    assert!(!text.contains("table: memories"));
}

// Interleaved batch edits must pair bodies by node identity rather than row order.
#[test]
fn batch_pairs_interleaved_nodes_by_uuid() {
    let s = Store::memory().unwrap();
    create(&s, "one-start\n");
    s.write("a", None, |c| {
        graph::create(c, "a", "core://", "two", "two-start\n", 0, None)
    })
    .unwrap();
    edit(&s, "intermediate\n", Some("multi"));
    s.write("a", Some("multi"), |c| {
        graph::update(c, "a", "core://two", "two-end\n", None)
    })
    .unwrap();
    edit(&s, "one-end\n", Some("multi"));
    let text = changes::diff_text(s.connection(), "a", "multi", true).unwrap();
    assert!(text.contains("-one-start\n+one-end\n"), "{text}");
    assert!(text.contains("-two-start\n+two-end\n"), "{text}");
    assert!(!text.contains("intermediate"));
    assert_eq!(text.matches("content: ").count(), 2);
}
