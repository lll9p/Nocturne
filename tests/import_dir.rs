use nocturne::{error::Error, graph, import_dir, store::Store};
use std::{fs, path::PathBuf};

struct Source(PathBuf);
impl Source {
    fn new() -> Self {
        let path = std::env::temp_dir()
            .join(format!("nocturne-import-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn count(s: &Store, table: &str) -> i64 {
    s.connection()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

// A stale parent or a failure after the first insertion must roll back the entire import.
#[test]
fn parent_replacement_and_midway_failure_are_atomic() {
    let source = Source::new();
    fs::write(source.0.join("a.txt"), "a").unwrap();
    fs::write(source.0.join("b.txt"), "b").unwrap();
    let s = Store::memory().unwrap();
    s.write("", None, |c| {
        graph::create(c, "", "core://", "parent", "", 0, None)
    })
    .unwrap();
    let plan = import_dir::plan(s.connection(), "", &source.0, "core://parent")
        .unwrap();
    s.connection().execute_batch("CREATE TRIGGER fail_import BEFORE INSERT ON edges WHEN NEW.name='b' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    let before = count(&s, "nodes");
    assert!(
        s.write("", None, |c| import_dir::apply(c, "", &plan, None))
            .is_err()
    );
    assert_eq!(count(&s, "nodes"), before);
    assert!(graph::resolve(s.connection(), "", "core://parent/a").is_err());
    s.connection()
        .execute_batch("DROP TRIGGER fail_import")
        .unwrap();
    s.write("", None, |c| {
        graph::create(c, "", "other://", "replacement", "", 0, None)
    })
    .unwrap();
    let replacement =
        graph::resolve(s.connection(), "", "other://replacement").unwrap();
    // Simulate replacement independently of the derived search index.
    s.connection()
        .execute(
            "UPDATE paths SET node_uuid=?1 WHERE namespace='' AND domain='core' AND path='parent'",
            [&replacement],
        )
        .unwrap();
    assert!(matches!(
        s.write("", None, |c| import_dir::apply(c, "", &plan, None)),
        Err(Error::Conflict(_))
    ));
}

// Preflight must collect errors below invalid parents and reject the whole tree without audit writes.
#[test]
fn preflight_collects_invalid_descendants_collisions_and_encoding() {
    let source = Source::new();
    fs::create_dir(source.0.join("bad dir")).unwrap();
    fs::write(source.0.join("bad dir/child!.txt"), "x").unwrap();
    fs::write(source.0.join("bad dir/child.txt"), "x").unwrap();
    fs::write(source.0.join("bad name.txt"), "x").unwrap();
    fs::write(source.0.join("broken.txt"), [255]).unwrap();
    for name in ["one", "two"] {
        fs::create_dir(source.0.join(name)).unwrap();
        fs::write(source.0.join(format!("{name}.md")), "x").unwrap();
    }
    fs::write(source.0.join("good.txt"), "x").unwrap();
    let s = Store::memory().unwrap();
    let plan = import_dir::plan_with_sanitize(
        s.connection(),
        "",
        &source.0,
        "core://",
        false,
    )
    .unwrap();
    assert!(!plan.is_valid());
    let preview = plan.preview();
    assert!(preview["issue_count"].as_u64().unwrap() >= 8);
    // Keep bounded, structured diagnostics without embedding them again in the summary.
    let Error::ImportValidation {
        message,
        issue_count,
        problem_source_count,
        samples,
        ..
    } = plan.validate().unwrap_err()
    else {
        panic!("preflight must return structured validation errors");
    };
    assert_eq!(issue_count as u64, preview["issue_count"]);
    assert_eq!(problem_source_count as u64, preview["problem_source_count"]);
    assert_eq!(samples, preview["issues"].as_array().unwrap()[..3]);
    assert!(!message.contains(samples[0]["source"].as_str().unwrap()));
    assert!(message.contains(&issue_count.to_string()));
    let errors = preview["issues"].to_string();
    for name in [
        "bad dir", "child!", "bad name", "broken", "one.md", "two.md",
    ] {
        assert!(errors.contains(name), "{errors}");
    }
    assert!(
        !preview["mappings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["uri"].as_str().unwrap().contains("child"))
    );
    assert_eq!(count(&s, "changesets"), 0);
    let before = count(&s, "nodes");
    assert!(
        s.write("", None, |c| import_dir::apply(c, "", &plan, None))
            .is_err()
    );
    assert_eq!(count(&s, "nodes"), before);
    assert_eq!(count(&s, "changesets"), 0);
}

// Sanitization must report every original source that maps to the same destination.
#[test]
fn sanitize_preserves_each_original_collision_source() {
    let source = Source::new();
    for name in ["a b.txt", "a.b.md", "a-b.txt", "x!.txt"] {
        fs::write(source.0.join(name), "x").unwrap();
    }
    let s = Store::memory().unwrap();
    let plan = import_dir::plan_with_sanitize(
        s.connection(),
        "",
        &source.0,
        "core://",
        true,
    )
    .unwrap();
    let preview = plan.preview();
    assert_eq!(preview["issue_count"], 3);
    let message = preview["issues"].to_string();
    for name in ["a b.txt", "a.b.md", "a-b.txt"] {
        assert!(message.contains(name), "{message}");
    }
    assert_eq!(preview["mappings"].as_array().unwrap().len(), 4);
    assert_eq!(count(&s, "changesets"), 0);
}

// Parent bodies, node attrs and shared edges must survive without copying source identities or external aliases.
#[test]
fn manifest_roundtrip_preserves_graph_and_metadata() {
    use nocturne::{attrs, export_dir::export_with_options, manifest};
    use serde_json::json;
    let s = Store::memory().unwrap();
    s.write("source", None, |c| {
        graph::create(c, "source", "lit://", "docs", "parent\r\n", 7, None)?;
        graph::create(c, "source", "lit://docs", "leaf", "leaf", 9, Some(""))?;
        attrs::set(
            c,
            "source",
            "lit://docs",
            &json!({"tag":"parent", "source_path":"原始 (书籍).md"}),
        )?;
        graph::alias_with_meta(
            c,
            "source",
            "lit://docs",
            "lit://copy",
            Some(3),
            Some("copy view"),
        )?;
        // An unlinked derived child replaced by a distinct child under the alias would collide if manifest::apply ran derive().
        graph::create(c, "source", "lit://", "a", "A", 0, None)?;
        graph::create(c, "source", "lit://a", "other", "X", 0, None)?;
        graph::create(c, "source", "lit://", "b", "B", 0, None)?;
        graph::alias(c, "source", "lit://a", "lit://b/alias")?;
        graph::unlink(c, "source", "lit://b/alias/other")?;
        graph::create(c, "source", "lit://b/alias", "other", "Y", 0, None)
    })
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("out");
    export_with_options(s.connection(), "source", &out, Some("lit://"), true)
        .unwrap();
    let target = Store::memory().unwrap();
    target
        .write("target", None, |c| {
            graph::create(
                c,
                "target",
                "other://",
                "untouched",
                "existing",
                0,
                None,
            )
        })
        .unwrap();
    let original =
        graph::resolve(target.connection(), "target", "other://untouched")
            .unwrap();
    let p = manifest::plan(
        target.connection(),
        "target",
        &out,
        PathBuf::from("nocturne-manifest.json").as_path(),
    )
    .unwrap();
    assert_eq!(p.preview()["valid"], true);
    // A savepoint alone must not permit an unaudited autocommit import.
    assert!(manifest::apply(target.connection(), "target", &p).is_err());
    assert_eq!(count(&target, "edges"), 1);
    target
        .write("target", None, |c| manifest::apply(c, "target", &p))
        .unwrap();
    let c = target.connection();
    let parent = graph::read(c, "target", "lit://docs", false).unwrap();
    assert_eq!(parent["content"], "parent\r\n");
    assert_eq!(
        parent["attrs"],
        json!({"tag":"parent", "source_path":"原始 (书籍).md"})
    );
    assert_eq!(parent["disclosure"], serde_json::Value::Null);
    assert_eq!(
        graph::read(c, "target", "lit://copy/leaf", false).unwrap()["disclosure"],
        ""
    );
    assert_eq!(
        graph::resolve(c, "target", "lit://copy").unwrap(),
        graph::resolve(c, "target", "lit://docs").unwrap()
    );
    assert_ne!(
        graph::resolve(c, "target", "lit://docs").unwrap(),
        graph::resolve(s.connection(), "source", "lit://docs").unwrap()
    );
    let edges: i64 = c
        .query_row("SELECT count(*) FROM edges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(edges, 9);
    let copy = graph::read(c, "target", "lit://copy", false).unwrap();
    assert_eq!(copy["priority"], 3);
    assert_eq!(copy["disclosure"], "copy view");
    let mut snapshots = Vec::new();
    for (store, ns) in [(&s, "source"), (&target, "target")] {
        snapshots.push(store.connection().prepare("SELECT path FROM paths WHERE namespace=?1 AND domain='lit' ORDER BY path").unwrap().query_map([ns],|r|r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap());
    }
    assert_eq!(snapshots[0], snapshots[1]);
    assert_eq!(
        graph::resolve(c, "target", "other://untouched").unwrap(),
        original
    );
    assert_eq!(
        graph::read(c, "target", "other://untouched", false).unwrap()["content"],
        "existing"
    );
    assert_eq!(
        graph::read(c, "target", "lit://a/other", false).unwrap()["content"],
        "X"
    );
    assert_eq!(
        graph::read(c, "target", "lit://b/alias/other", false).unwrap()["content"],
        "Y"
    );
    let names: Vec<String> = c
        .prepare("SELECT name FROM edges ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        names,
        vec![
            "a",
            "alias",
            "b",
            "copy",
            "docs",
            "leaf",
            "other",
            "other",
            "untouched"
        ]
    );
    assert_eq!(parent["priority"], 7);
    assert_eq!(
        graph::read(c, "target", "lit://copy/leaf", false).unwrap()["priority"],
        9
    );
}

// Invalid references, replaced source files and post-plan occupancy must reject the whole batch.
#[test]
fn manifest_rejects_untrusted_sources_and_stale_plans() {
    use nocturne::{export_dir::export_with_options, manifest};
    let source = Store::memory().unwrap();
    source
        .write("", None, |c| {
            graph::create(c, "", "lit://", "a", "a", 0, None)?;
            graph::create(c, "", "lit://", "b", "b", 0, None)
        })
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    export_with_options(source.connection(), "", &out, None, true).unwrap();
    let file = out.join("nocturne-manifest.json");
    let original = fs::read(&file).unwrap();
    let target = Store::memory().unwrap();
    for reference in ["../outside.md", "/outside.md"] {
        let mut value: serde_json::Value =
            serde_json::from_slice(&original).unwrap();
        value["paths"][0]["file"] = reference.into();
        fs::write(&file, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(manifest::plan(target.connection(), "", &out, &file).is_err());
        assert_eq!(count(&target, "edges"), 0);
    }
    for mutate in ["unknown", "version", "uri"] {
        let mut value: serde_json::Value =
            serde_json::from_slice(&original).unwrap();
        match mutate {
            "unknown" => value["ignored_metadata"] = true.into(),
            "version" => value["version"] = 2.into(),
            _ => value["paths"][0]["uri"] = "lit://../escape".into(),
        }
        fs::write(&file, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(manifest::plan(target.connection(), "", &out, &file).is_err());
    }
    fs::write(&file, &original).unwrap();
    let p = manifest::plan(target.connection(), "", &out, &file).unwrap();
    fs::write(out.join("lit/a.md"), "replaced").unwrap();
    assert!(
        target
            .write("", None, |c| manifest::apply(c, "", &p))
            .is_err()
    );
    assert_eq!(count(&target, "edges"), 0);
    fs::write(out.join("lit/a.md"), "a").unwrap();
    let p = manifest::plan(target.connection(), "", &out, &file).unwrap();
    target.connection().execute_batch("CREATE TRIGGER fail_manifest BEFORE INSERT ON edges WHEN NEW.name='b' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(
        target
            .write("", None, |c| manifest::apply(c, "", &p))
            .is_err()
    );
    assert_eq!(count(&target, "edges"), 0);
    target
        .connection()
        .execute_batch("DROP TRIGGER fail_manifest")
        .unwrap();
    target
        .write("", None, |c| {
            graph::create(c, "", "lit://", "b", "occupied", 0, None)
        })
        .unwrap();
    assert!(
        target
            .write("", None, |c| manifest::apply(c, "", &p))
            .is_err()
    );
    assert!(graph::resolve(target.connection(), "", "lit://a").is_err());
}

// Nested exports cannot invent out-of-scope ancestors; replacing a preflighted parent invalidates the plan.
#[test]
fn manifest_requires_and_rechecks_external_parent() {
    use nocturne::{export_dir::export_with_options, manifest};
    let source = Store::memory().unwrap();
    source
        .write("", None, |c| {
            graph::create(c, "", "lit://", "docs", "not exported", 0, None)?;
            graph::create(c, "", "lit://docs", "empty", "", 0, None)?;
            graph::create(c, "", "lit://docs/empty", "leaf", "body", 0, None)
        })
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("out");
    export_with_options(
        source.connection(),
        "",
        &out,
        Some("lit://docs/empty"),
        true,
    )
    .unwrap();
    let target = Store::memory().unwrap();
    let file = out.join("nocturne-manifest.json");
    let p = manifest::plan(target.connection(), "", &out, &file).unwrap();
    assert_eq!(p.preview()["valid"], false);
    assert!(p.validate().is_err());
    target
        .write("", None, |c| {
            graph::create(c, "", "lit://", "docs", "existing", 0, None)
        })
        .unwrap();
    let p = manifest::plan(target.connection(), "", &out, &file).unwrap();
    target
        .write("", None, |c| {
            graph::create(c, "", "lit://", "replacement", "", 0, None)
        })
        .unwrap();
    target.connection().execute_batch("UPDATE paths SET node_uuid=(SELECT node_uuid FROM paths WHERE domain='lit' AND path='replacement') WHERE domain='lit' AND path='docs';").unwrap();
    assert!(
        target
            .write("", None, |c| manifest::apply(c, "", &p))
            .is_err()
    );
    assert!(
        graph::resolve(target.connection(), "", "lit://docs/empty").is_err()
    );
    let p = manifest::plan(target.connection(), "", &out, &file).unwrap();
    target
        .write("", None, |c| manifest::apply(c, "", &p))
        .unwrap();
    assert_eq!(
        graph::read(target.connection(), "", "lit://docs/empty", false)
            .unwrap()["content"],
        ""
    );
}

// Large conflict sets remain complete in dry-run while the ordinary error retains only three examples.
#[test]
fn manifest_conflict_summary_is_bounded() {
    use nocturne::{export_dir::export_with_options, manifest};
    let source = Store::memory().unwrap();
    let target = Store::memory().unwrap();
    for i in 0..8 {
        for s in [&source, &target] {
            s.write("", None, |c| {
                graph::create(
                    c,
                    "",
                    "lit://",
                    &format!("note{i}"),
                    "body",
                    0,
                    None,
                )
            })
            .unwrap();
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("out");
    export_with_options(source.connection(), "", &out, None, true).unwrap();
    let p = manifest::plan(
        target.connection(),
        "",
        &out,
        &out.join("nocturne-manifest.json"),
    )
    .unwrap();
    assert_eq!(p.preview()["issues"].as_array().unwrap().len(), 8);
    match p.validate().unwrap_err() {
        Error::ImportValidation {
            code,
            issue_count,
            problem_source_count,
            samples,
            message,
        } => {
            assert_eq!(
                (code, issue_count, problem_source_count, samples.len()),
                (4, 8, 8, 3)
            );
            assert!(message.len() < 100);
        }
        error => panic!("unexpected validation error: {error}"),
    }
}

// Even a symlink pointing back inside the export is rejected instead of being followed.
#[cfg(unix)]
#[test]
fn manifest_rejects_symlink_sources_at_plan_and_apply() {
    use nocturne::{export_dir::export_with_options, manifest};
    let source = Store::memory().unwrap();
    source
        .write("", None, |c| {
            graph::create(c, "", "lit://", "a", "body", 0, None)
        })
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    export_with_options(source.connection(), "", &out, None, true).unwrap();
    let target = Store::memory().unwrap();
    let file = out.join("nocturne-manifest.json");
    let p = manifest::plan(target.connection(), "", &out, &file).unwrap();
    fs::rename(out.join("lit/a.md"), out.join("original.md")).unwrap();
    std::os::unix::fs::symlink("../original.md", out.join("lit/a.md")).unwrap();
    assert!(manifest::plan(target.connection(), "", &out, &file).is_err());
    assert!(
        target
            .write("", None, |c| manifest::apply(c, "", &p))
            .is_err()
    );
    assert_eq!(count(&target, "edges"), 0);
}

// Source provenance must survive sanitization without replacing shared metadata or leaking host paths.
#[test]
fn source_paths_preserve_input_names_and_reject_attribute_collisions() {
    use serde_json::json;
    let source = Source::new();
    fs::create_dir(source.0.join("资料 (原稿)")).unwrap();
    fs::write(source.0.join("资料 (原稿)/01 概述.md"), "original\r\n").unwrap();
    let s = Store::memory().unwrap();
    let plan = import_dir::plan_with_sanitize(
        s.connection(),
        "",
        &source.0,
        "lit://",
        true,
    )
    .unwrap();
    let shared = json!({"collection":"papers"});
    let plan = plan
        .with_source_path_attr("source_path", Some(&shared))
        .unwrap();
    let preview = plan.preview();
    assert_eq!(preview["source_path_attr"], "source_path");
    assert_eq!(preview["mappings"][0]["source_path"], "资料 (原稿)");
    assert_eq!(
        preview["mappings"][1]["source_path"],
        "资料 (原稿)/01 概述.md"
    );
    let before = count(&s, "nodes");
    assert!(matches!(
        s.write("", None, |c| import_dir::apply(
            c,
            "",
            &plan,
            Some(&json!({"source_path":null}))
        )),
        Err(Error::Invalid(_))
    ));
    assert_eq!(count(&s, "nodes"), before);
    s.write("", None, |c| import_dir::apply(c, "", &plan, Some(&shared)))
        .unwrap();
    for (uri, expected) in [
        ("lit://资料-原稿", "资料 (原稿)"),
        ("lit://资料-原稿/01-概述", "资料 (原稿)/01 概述.md"),
    ] {
        let attrs = nocturne::attrs::get(s.connection(), "", uri).unwrap();
        assert_eq!(attrs["attrs"]["source_path"], expected);
        assert_eq!(attrs["attrs"]["collection"], "papers");
    }
    for key in ["", " "] {
        assert!(
            import_dir::plan_with_sanitize(
                s.connection(),
                "",
                &source.0,
                "other://",
                true
            )
            .unwrap()
            .with_source_path_attr(key, None)
            .is_err()
        );
    }
    let warning_plan = import_dir::plan_with_sanitize(
        s.connection(),
        "",
        &source.0,
        "warn://",
        true,
    )
    .unwrap()
    .with_source_path_attr("priority", None)
    .unwrap();
    assert_eq!(warning_plan.warnings(None).len(), 1);
    let result = s
        .write("", None, |c| import_dir::apply(c, "", &warning_plan, None))
        .unwrap();
    assert_eq!(result["warnings"].as_array().unwrap().len(), 1);
    assert_eq!(
        nocturne::attrs::get(s.connection(), "", "warn://资料-原稿").unwrap()["attrs"]
            ["priority"],
        "资料 (原稿)"
    );
    let fresh = import_dir::plan_with_sanitize(
        s.connection(),
        "",
        &source.0,
        "other://",
        true,
    )
    .unwrap();
    assert!(
        fresh
            .with_source_path_attr(
                "source_path",
                Some(&json!({"source_path":null}))
            )
            .is_err()
    );
}
