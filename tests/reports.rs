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
