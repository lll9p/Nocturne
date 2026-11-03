use crate::{
    error::{Error, Result},
    uri::Uri,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

fn name(value: &str) -> Result<&str> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(Error::Invalid("invalid preset name".into()));
    }
    Ok(value.trim())
}

fn validate_boot(value: &Value) -> Result<()> {
    let map = value.as_object().ok_or_else(|| {
        Error::Invalid("boot_uris must be a namespace map".into())
    })?;
    for (ns, uris) in map {
        if ns.chars().any(char::is_control) {
            return Err(Error::Invalid("invalid namespace".into()));
        }
        let uris = uris.as_array().ok_or_else(|| {
            Error::Invalid("boot_uris values must be string arrays".into())
        })?;
        for uri in uris {
            Uri::parse(uri.as_str().ok_or_else(|| {
                Error::Invalid("boot URI must be a string".into())
            })?)?;
        }
    }
    Ok(())
}

type PresetRow = (i64, String, String, Option<String>, bool, String, String);

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PresetRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
    ))
}
fn serialize(r: PresetRow) -> Result<Value> {
    Ok(
        json!({"id":r.0,"name":r.1,"boot_uris":serde_json::from_str::<Value>(&r.2)?,
        "path_masks":r.3.map(|s| serde_json::from_str::<Value>(&s)).transpose()?,
        "is_active":r.4,"created_at":r.5,"updated_at":r.6}),
    )
}
const COLUMNS: &str =
    "id,name,boot_uris,path_masks,is_active,created_at,updated_at";

pub fn list(c: &Connection) -> Result<Value> {
    let mut q = c.prepare(&format!(
        "SELECT {COLUMNS} FROM presets ORDER BY created_at,id"
    ))?;
    let rows = q
        .query_map([], row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(json!(
        rows.into_iter()
            .map(serialize)
            .collect::<Result<Vec<_>>>()?
    ))
}
pub fn show(c: &Connection, preset_name: &str) -> Result<Value> {
    let preset_name = name(preset_name)?;
    let found = c
        .query_row(
            &format!("SELECT {COLUMNS} FROM presets WHERE name=?1"),
            [preset_name],
            row,
        )
        .optional()?;
    serialize(found.ok_or_else(|| {
        Error::NotFound(format!("preset not found: {preset_name}"))
    })?)
}
fn available(c: &Connection, name: &str, except: Option<i64>) -> Result<()> {
    let exists: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM presets WHERE name=?1 AND (?2 IS NULL OR id<>?2))",
        params![name, except],
        |r| r.get(0),
    )?;
    if exists {
        return Err(Error::Conflict(format!("preset already exists: {name}")));
    }
    Ok(())
}

/// Mutations must run inside the caller's Store::write transaction.
pub fn create(
    c: &Connection, preset_name: &str, boot_uris: &Value, make_active: bool,
) -> Result<Value> {
    let preset_name = name(preset_name)?;
    validate_boot(boot_uris)?;
    available(c, preset_name, None)?;
    c.execute(
        "INSERT INTO presets(name,boot_uris) VALUES(?1,?2)",
        params![preset_name, boot_uris.to_string()],
    )?;
    if make_active {
        activate(c, preset_name)
    } else {
        show(c, preset_name)
    }
}
pub fn update(
    c: &Connection, preset_name: &str, new_name: Option<&str>,
    boot_uris: Option<&Value>,
) -> Result<Value> {
    if new_name.is_none() && boot_uris.is_none() {
        return Err(Error::Invalid(
            "preset update requires a name or boot map".into(),
        ));
    }
    let old = show(c, preset_name)?;
    let target = name(new_name.unwrap_or(old["name"].as_str().unwrap()))?;
    available(c, target, old["id"].as_i64())?;
    if let Some(boot) = boot_uris {
        validate_boot(boot)?;
    }
    c.execute("UPDATE presets SET name=?1,boot_uris=coalesce(?2,boot_uris),updated_at=CURRENT_TIMESTAMP WHERE id=?3",
        params![target, boot_uris.map(Value::to_string), old["id"].as_i64()])?;
    show(c, target)
}
pub fn duplicate(
    c: &Connection, preset_name: &str, new_name: &str,
) -> Result<Value> {
    let old = show(c, preset_name)?;
    let target = name(new_name)?;
    available(c, target, None)?;
    // Copy the stored JSON text verbatim, including uninterpreted path masks.
    c.execute("INSERT INTO presets(name,boot_uris,path_masks) SELECT ?1,boot_uris,path_masks FROM presets WHERE id=?2", params![target,old["id"].as_i64()])?;
    show(c, target)
}
pub fn delete(c: &Connection, preset_name: &str) -> Result<Value> {
    let old = show(c, preset_name)?;
    if old["is_active"] == true {
        return Err(Error::Conflict("cannot delete active preset".into()));
    }
    c.execute(
        "DELETE FROM presets WHERE id=?1",
        [old["id"].as_i64().unwrap()],
    )?;
    Ok(old)
}
pub fn activate(c: &Connection, preset_name: &str) -> Result<Value> {
    let old = show(c, preset_name)?;
    if old["is_active"] == true {
        return Ok(old);
    }
    c.execute(
        "UPDATE presets SET is_active=0,updated_at=CURRENT_TIMESTAMP WHERE is_active=1",
        [],
    )?;
    c.execute(
        "UPDATE presets SET is_active=1,updated_at=CURRENT_TIMESTAMP WHERE id=?1",
        [old["id"].as_i64().unwrap()],
    )?;
    show(c, preset_name)
}
