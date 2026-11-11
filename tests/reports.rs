use nocturne::reports::{select_weighted, selection_weight};

// Application-defined staleness and priority weights must determine the half-open selection intervals.
#[test]
fn staleness_priority_weights_control_selection() {
    let weights = [selection_weight(12.0, 2), selection_weight(6.0, 0)];
    assert_eq!(weights, [3.0, 6.0]);
    assert_eq!(select_weighted(&weights, 0.0), Some(0));
    assert_eq!(select_weighted(&weights, 0.32), Some(0));
    assert_eq!(select_weighted(&weights, 1.0 / 3.0), Some(1));
    assert_eq!(select_weighted(&weights, 0.999999), Some(1));
    assert_eq!(selection_weight(-2.0, -1), 0.5);
    assert_eq!(selection_weight(10.0, 10000), 1e-11);
}

// Unvisited libraries audited only with --no-track must still surface stale nodes by creation age and report broken parent routes.
#[test]
fn diagnostic_detects_stale_unvisited_nodes_and_broken_parent_routes() {
    use nocturne::{graph, reports::diagnostic, store::Store};
    let s = Store::memory().unwrap();
    let created = s
        .write("", None, |c| {
            graph::create(c, "", "core://", "aged", "never visited", 0, None)
        })
        .unwrap();
    let c = s.connection();
    c.execute(
        "UPDATE nodes SET created_at=datetime('now','-10 days') WHERE uuid=?1",
        [created["node_uuid"].as_str().unwrap()],
    )
    .unwrap();
    c.execute(
        "INSERT INTO paths(namespace,domain,path,edge_id,node_uuid)
         SELECT '','core','missing_parent/child',edge_id,node_uuid FROM paths WHERE path='aged'",
        [],
    )
    .unwrap();
    let report = diagnostic(c, "", Some("core"), 30, 10, 10, 2048).unwrap();
    assert_eq!(report["stale_nodes"].as_array().unwrap().len(), 1);
    assert_eq!(report["stale_nodes"][0]["uuid"], created["node_uuid"]);
    assert_eq!(report["broken_parent_routes"].as_array().unwrap().len(), 1);
    assert_eq!(
        report["broken_parent_routes"][0]["uri"],
        "core://missing_parent/child"
    );
}
