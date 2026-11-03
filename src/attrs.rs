use crate::{
    error::{Error, Result},
    graph,
    store::ROOT,
    uri::Uri,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

pub fn for_node(c: &Connection, node: &str) -> Result<Value> {
    let data: Option<String> = c
        .query_row(
            "SELECT data_json FROM node_attrs WHERE node_uuid=?1",
            [node],
            |r| r.get(0),
        )
        .optional()?;
    Ok(data
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_else(|| json!({})))
}

pub fn get(c: &Connection, ns: &str, uri: &str) -> Result<Value> {
    let node = graph::resolve(c, ns, uri)?;
    Ok(json!({"uri":uri,"node_uuid":node,"attrs":for_node(c, &node)?}))
}

pub fn set(
    c: &Connection, ns: &str, uri: &str, value: &Value,
) -> Result<Value> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::Invalid("attrs must be an object".into()))?;
    if Uri::parse(uri)?.domain == "system" {
        return Err(Error::Invalid("system domain is reserved".into()));
    }
    let node = graph::resolve(c, ns, uri)?;
    if node == ROOT {
        return Err(Error::Invalid("ROOT cannot have attrs".into()));
    }
    if object.is_empty() {
        c.execute("DELETE FROM node_attrs WHERE node_uuid=?1", [&node])?;
    } else {
        let data = serde_json::to_string(value)?;
        c.execute("INSERT INTO node_attrs(node_uuid,data_json) VALUES(?1,?2) ON CONFLICT(node_uuid) DO UPDATE SET data_json=excluded.data_json WHERE data_json<>excluded.data_json", params![node, data])?;
    }
    let mut result = json!({"uri":uri,"node_uuid":node,"attrs":value});
    let warnings = warnings(value);
    if !warnings.is_empty() {
        result["warnings"] = json!(warnings);
    }
    Ok(result)
}

pub fn parse_assignment(input: &str) -> Result<(String, String)> {
    let (key, value) = input.split_once('=').ok_or_else(|| {
        Error::Invalid("attrs assignment must be key=value".into())
    })?;
    if key.is_empty() {
        return Err(Error::Invalid("attrs key cannot be empty".into()));
    }
    Ok((key.to_owned(), value.to_owned()))
}

pub fn warnings(value: &Value) -> Vec<String> {
    ["priority", "disclosure"].into_iter()
        .filter(|key| value.as_object().is_some_and(|obj| obj.contains_key(*key)))
        .map(|key| format!("attrs.{key} is only a custom node field; it does not set edge {key}. Use meta/create/import-dir --{key} instead."))
        .collect()
}

pub fn edit(
    c: &Connection, ns: &str, uri: &str, sets: &[(String, String)],
    unsets: &[String],
) -> Result<Value> {
    let mut keys = std::collections::HashSet::new();
    for (key, _) in sets {
        if key.is_empty() || !keys.insert(key) {
            return Err(Error::Invalid(
                "attrs set keys must be nonempty and unique".into(),
            ));
        }
    }
    if unsets.iter().any(|key| keys.contains(key)) {
        return Err(Error::Invalid(
            "cannot set and unset the same attrs key".into(),
        ));
    }
    let node = graph::resolve(c, ns, uri)?;
    let mut value = for_node(c, &node)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| Error::Invalid("attrs must be an object".into()))?;
    for (key, value) in sets {
        object.insert(key.clone(), Value::String(value.clone()));
    }
    for key in unsets {
        object.remove(key);
    }
    set(c, ns, uri, &value)
}
