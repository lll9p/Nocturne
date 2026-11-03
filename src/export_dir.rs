//! A plaintext URI view, not a lossless round-trip format for import-dir.
use crate::error::{Error, Result};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
use unicode_normalization::UnicodeNormalization;

fn component(name: &str) -> Result<()> {
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_uppercase();
    let device = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|p| {
        stem.strip_prefix(p).is_some_and(|s| {
            matches!(
                s,
                "1" | "2"
                    | "3"
                    | "4"
                    | "5"
                    | "6"
                    | "7"
                    | "8"
                    | "9"
                    | "¹"
                    | "²"
                    | "³"
            )
        })
    });
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        || device
        || name.len() > 255
        || name.encode_utf16().count() > 255
    {
        return Err(Error::Invalid(format!(
            "invalid export path component: {name:?}"
        )));
    }
    Ok(())
}

// Includes implicit directories so file/directory and case collisions cannot hide.
fn add(
    entries: &mut BTreeMap<String, (PathBuf, Option<i64>)>, path: PathBuf,
    content: Option<i64>,
) -> Result<()> {
    let key = path
        .to_string_lossy()
        .nfc()
        .flat_map(char::to_uppercase)
        .collect::<String>();
    if let Some((previous, body)) = entries.get(&key) {
        if previous != &path || body.is_some() || content.is_some() {
            return Err(Error::Conflict(format!(
                "export path collision: {} and {}",
                previous.display(),
                path.display()
            )));
        }
    } else {
        entries.insert(key, (path, content));
    }
    Ok(())
}

fn io_error(path: &Path, error: io::Error) -> Error {
    let message = format!("{}: {error}", path.display());
    match error.kind() {
        io::ErrorKind::AlreadyExists | io::ErrorKind::DirectoryNotEmpty => {
            Error::Conflict(message)
        }
        _ => Error::Storage(message),
    }
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn publish(source: &Path, target: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    #[cfg(target_os = "linux")]
    unsafe extern "C" {
        fn renameat2(
            a: i32, b: *const std::ffi::c_char, c: i32,
            d: *const std::ffi::c_char, flags: u32,
        ) -> i32;
    }
    #[cfg(target_os = "macos")]
    unsafe extern "C" {
        fn renamex_np(
            from: *const std::ffi::c_char, to: *const std::ffi::c_char,
            flags: u32,
        ) -> i32;
    }
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|e| Error::Invalid(e.to_string()))?;
    let target_c = CString::new(target.as_os_str().as_bytes())
        .map_err(|e| Error::Invalid(e.to_string()))?;
    // Unlike rename(), both exclusive primitives protect competing empty directories.
    #[cfg(target_os = "linux")]
    let result =
        unsafe { renameat2(-100, source.as_ptr(), -100, target_c.as_ptr(), 1) }; // RENAME_NOREPLACE
    #[cfg(target_os = "macos")]
    let result =
        unsafe { renamex_np(source.as_ptr(), target_c.as_ptr(), 0x00000004) }; // RENAME_EXCL
    if result != 0 {
        return Err(io_error(target, io::Error::last_os_error()));
    }
    Ok(())
}
#[cfg(windows)]
fn publish(source: &Path, target: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            source: *const u16, target: *const u16, flags: u32,
        ) -> i32;
    }
    let wide = |path: &Path| -> Result<Vec<u16>> {
        let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err(Error::Invalid("NUL in export path".into()));
        }
        value.push(0);
        Ok(value)
    };
    let source = wide(source)?;
    let target_w = wide(target)?;
    // No MOVEFILE_REPLACE_EXISTING: std::fs::rename permits replacing empty directories.
    if unsafe { MoveFileExW(source.as_ptr(), target_w.as_ptr(), 0) } == 0 {
        return Err(io_error(target, io::Error::last_os_error()));
    }
    Ok(())
}
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn publish(_: &Path, _: &Path) -> Result<()> {
    Err(Error::Invalid(
        "atomic no-replace directory export is supported on Windows, Linux and macOS".into(),
    ))
}

/// Export only the selected namespace, without recording reads or changesets.
/// Reject caller-owned transactions rather than replacing their snapshot.
pub fn export(
    conn: &Connection, namespace: &str, output: &Path,
) -> Result<Value> {
    if !conn.is_autocommit() {
        return Err(Error::Invalid(
            "export-dir requires a connection without an active transaction"
                .into(),
        ));
    }
    match fs::symlink_metadata(output) {
        Ok(_) => {
            return Err(io_error(output, io::ErrorKind::AlreadyExists.into()));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let name = output
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::Invalid("invalid export destination".into()))?;
    component(name)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // Canonicalizing the existing parent also makes staging and publication use the same directory.
    let parent = fs::canonicalize(parent)?;
    let target = parent.join(name);
    let tx = conn.unchecked_transaction()?;
    let rows = {
        let mut stmt = tx.prepare("SELECT p.domain,p.path,m.id,length(CAST(m.content AS BLOB)) FROM paths p JOIN memories m ON m.node_uuid=p.node_uuid AND m.deprecated=0 WHERE p.namespace=?1 ORDER BY p.domain COLLATE BINARY,p.path COLLATE BINARY")?;
        let rows = stmt.query_map([namespace], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    let parents: BTreeSet<_> = rows
        .iter()
        .flat_map(|(domain, path, _, _)| {
            path.match_indices('/')
                .map(move |(i, _)| (domain.as_str(), &path[..i]))
        })
        .collect();
    let mut entries = BTreeMap::new();
    for (domain, path, memory_id, bytes) in &rows {
        component(domain)?;
        let mut dest = PathBuf::from(domain);
        add(&mut entries, dest.clone(), None)?;
        let parts = path.split('/').collect::<Vec<_>>();
        for (i, part) in parts.iter().enumerate() {
            component(part)?;
            dest.push(part);
            if i + 1 < parts.len() {
                add(&mut entries, dest.clone(), None)?;
            }
        }
        let parent_node = parents.contains(&(domain.as_str(), path.as_str()));
        if parent_node {
            add(&mut entries, dest.clone(), None)?;
        }
        if *bytes != 0 || !parent_node {
            let filename = format!("{}.md", parts.last().unwrap());
            component(&filename)?;
            dest.set_file_name(filename);
            add(&mut entries, dest, Some(*memory_id))?;
        }
    }
    let staging_path =
        parent.join(format!(".nocturne-export-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&staging_path).map_err(|e| io_error(&staging_path, e))?;
    let staging = Temporary(staging_path);
    let mut files = 0;
    let mut dirs = 0;
    // Prefix ordering puts explicit parents first; exclusive creation also rejects OS aliases.
    let mut content = tx.prepare("SELECT content FROM memories WHERE id=?1")?;
    for (path, memory_id) in entries.values() {
        let dest = staging.0.join(path);
        if let Some(memory_id) = memory_id {
            // Keep only the current body in memory; metadata and bodies share one snapshot.
            let body: String =
                content.query_row([memory_id], |row| row.get(0))?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&dest)
                .map_err(|e| io_error(&dest, e))?;
            file.write_all(body.as_bytes())
                .map_err(|e| io_error(&dest, e))?;
            files += 1;
        } else {
            fs::create_dir(&dest).map_err(|e| io_error(&dest, e))?;
            dirs += 1;
        }
    }
    drop(content);
    tx.commit()?;
    publish(&staging.0, &target)?;
    Ok(
        json!({"scope":"namespace", "namespace":namespace, "output":output, "files":files, "dirs":dirs}),
    )
}

#[cfg(all(test, any(windows, target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;

    // Protect a concurrent writer's output at the final, platform-specific rename boundary.
    #[test]
    fn publish_never_replaces_a_competing_target() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("staging");
        let target = temp.path().join("target");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("note.md"), "original").unwrap();
        // Simulates a target appearing after export's initial existence check.
        fs::create_dir(&target).unwrap();
        assert!(
            matches!(publish(&source, &target), Err(Error::Conflict(message)) if message.contains("target"))
        );
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        fs::remove_dir(&target).unwrap();
        fs::write(&target, "competing file").unwrap();
        assert!(
            matches!(publish(&source, &target), Err(Error::Conflict(message)) if message.contains("target"))
        );
        assert_eq!(fs::read(&target).unwrap(), b"competing file");
        fs::remove_file(&target).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(temp.path().join("missing"), &target)
                .unwrap();
            assert!(
                matches!(publish(&source, &target), Err(Error::Conflict(message)) if message.contains("target"))
            );
            assert!(
                fs::symlink_metadata(&target)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            fs::remove_file(&target).unwrap();
        }
        assert_eq!(fs::read(source.join("note.md")).unwrap(), b"original");
        publish(&source, &target).unwrap();
        assert!(!source.exists());
        assert_eq!(fs::read(target.join("note.md")).unwrap(), b"original");
    }
}
