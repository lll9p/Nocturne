use crate::error::{Error, Result};
use std::collections::BTreeSet;

fn normalize(text: &str) -> String {
    let mut out = String::new();
    let mut iter = text.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        if c == '\\'
            && (i == 0 || text.as_bytes()[i - 1] != b'\\')
            && iter.peek().is_some_and(|(_, c)| *c == 'n')
        {
            iter.next();
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

fn occurrences(text: &str, needle: &str) -> Vec<usize> {
    text.char_indices()
        .filter_map(|(i, _)| text[i..].starts_with(needle).then_some(i))
        .collect()
}

pub fn apply(content: &str, old: &str, new: &str) -> Result<String> {
    if old.is_empty() {
        return Err(Error::Invalid("empty match".into()));
    }
    let normalized = normalize(old);
    let mut spans = BTreeSet::new();
    for needle in [old, normalized.as_str()] {
        for i in occurrences(content, needle) {
            spans.insert((i, i + needle.len()));
        }
    }
    if let (Some(first), Some(last)) = (old.find("..."), old.rfind("...")) {
        let start = &old[..first];
        let end = &old[last + 3..];
        if !start.is_empty() && !end.is_empty() {
            for i in occurrences(content, start) {
                let offset = i + start.len();
                for j in occurrences(&content[offset..], end) {
                    spans.insert((i, offset + j + end.len()));
                    if spans.len() > 1 {
                        return Err(Error::PatchAmbiguous {
                            matches: spans.len(),
                        });
                    }
                }
            }
        }
    }
    if spans.is_empty() {
        return Err(Error::PatchNoMatch);
    }
    if spans.len() > 1 {
        return Err(Error::PatchAmbiguous {
            matches: spans.len(),
        });
    }
    let (start, end) = *spans.iter().next().unwrap();
    let replacement =
        if &content[start..end] != old && content[start..end] == normalized {
            normalize(new)
        } else {
            new.to_owned()
        };
    let result =
        format!("{}{}{}", &content[..start], replacement, &content[end..]);
    if result == content {
        return Err(Error::Invalid(
            "replacement produced identical content".into(),
        ));
    }
    Ok(result)
}
