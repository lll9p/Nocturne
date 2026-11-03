use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("Cargo.toml");
    let manifest: toml::Table = fs::read_to_string(manifest)
        .expect("read Cargo.toml")
        .parse()
        .expect("parse Cargo.toml");
    let version = manifest["package"]["metadata"]["nocturne"]["schema-version"]
        .as_integer()
        .expect("schema-version must be an integer");
    assert!(
        (1..=i64::from(i32::MAX)).contains(&version),
        "schema-version must be a positive SQLite user_version (signed 32-bit integer)"
    );
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(
        output.join("schema_version.rs"),
        format!("pub const SCHEMA_VERSION: i64 = {version};\n"),
    )
    .expect("write schema version");
}
