use crate::error::{Error, Result};
pub fn is_name_char(c: char) -> bool {
    c.is_alphanumeric()
        || c == '-'
        || c == '_'
        || unicode_normalization::char::is_combining_mark(c)
}

pub fn validate_name(name: &str, kind: &str) -> Result<()> {
    let invalid: String = name.chars().filter(|c| !is_name_char(*c)).collect();
    if name.is_empty() || !invalid.is_empty() {
        return Err(Error::Invalid(format!(
            "invalid {kind} {name:?}: illegal characters {invalid:?}; allowed: Unicode letters, digits, combining marks, '-' and '_' (nonempty)"
        )));
    }
    Ok(())
}
#[derive(Clone, Debug)]
pub struct Uri {
    pub domain: String,
    pub path: String,
}
impl Uri {
    pub fn parse(s: &str) -> Result<Self> {
        let (d, p) = s
            .split_once("://")
            .ok_or_else(|| Error::Invalid("expected domain://path".into()))?;
        validate_name(d, "domain")?;
        if s.chars().any(char::is_control)
            || (!p.is_empty()
                && p.split('/').any(|s| s.is_empty() || s == "." || s == ".."))
        {
            return Err(Error::Invalid("invalid URI".into()));
        }
        Ok(Self {
            domain: d.into(),
            path: p.into(),
        })
    }
    pub fn display(&self) -> String {
        format!("{}://{}", self.domain, self.path)
    }
}
