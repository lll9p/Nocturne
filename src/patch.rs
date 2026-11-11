use crate::error::{Error, Result};

fn occurrences(text: &str, needle: &str) -> Vec<usize> {
    text.char_indices()
        .filter_map(|(i, _)| text[i..].starts_with(needle).then_some(i))
        .collect()
}

pub fn apply(content: &str, old: &str, new: &str) -> Result<String> {
    if old.is_empty() {
        return Err(Error::Invalid("empty match".into()));
    }
    let matches = occurrences(content, old);
    if matches.is_empty() {
        return Err(Error::PatchNoMatch);
    }
    if matches.len() > 1 {
        return Err(Error::PatchAmbiguous {
            matches: matches.len(),
        });
    }
    let start = matches[0];
    let end = start + old.len();
    Ok(format!("{}{new}{}", &content[..start], &content[end..]))
}
