use crate::error::Result;
use rusqlite::Connection;
use serde_json::Value;

pub(crate) fn insert(c: &Connection, content: &str) -> Result<i64> {
    c.execute("INSERT INTO contents(content) VALUES(?1)", [content])?;
    Ok(c.last_insert_rowid())
}

pub(crate) fn expand(c: &Connection, mut row: Value) -> Result<Value> {
    if let Some(object) = row.as_object_mut()
        && let Some(id) = object.remove("content_id")
    {
        let content: String = c.query_row(
            "SELECT content FROM contents WHERE id=?1",
            [id.as_i64().ok_or_else(|| {
                crate::error::Error::Invalid("invalid content reference".into())
            })?],
            |r| r.get(0),
        )?;
        object.insert("content".into(), Value::String(content));
    }
    Ok(row)
}

pub(crate) fn collect(c: &Connection) -> Result<()> {
    c.execute("DELETE FROM contents WHERE NOT EXISTS(SELECT 1 FROM memories m WHERE m.content_id=contents.id) AND NOT EXISTS(SELECT 1 FROM change_rows r WHERE r.before_content_id=contents.id OR r.after_content_id=contents.id)", [])?;
    Ok(())
}
