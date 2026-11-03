use crate::error::Result;
use serde_json::{Value, json};
use similar::{ChangeTag, TextDiff};

pub fn compare(
    old: &str, new: &str, old_name: &str, new_name: &str,
) -> Result<Value> {
    let diff = TextDiff::from_lines(old, new);
    let (mut added, mut removed, mut unchanged) = (0usize, 0usize, 0usize);
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => added += 1,
            ChangeTag::Delete => removed += 1,
            ChangeTag::Equal => unchanged += 1,
        }
    }
    let unified = diff.unified_diff().header(old_name, new_name).to_string();
    Ok(json!({"unified":unified,"summary":{
        "added_lines":added,"removed_lines":removed,"unchanged_lines":unchanged,
        "old_lines":removed+unchanged,"new_lines":added+unchanged,"changed":old!=new
    }}))
}
