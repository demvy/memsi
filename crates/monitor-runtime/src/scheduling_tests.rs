use super::*;
use crate::tests::fixtures;

#[test]
fn watch_retention_expires_exact_boundary_and_orders_by_due_time_then_sequence() {
    let config = app_config::MonitorConfig::default();
    assert_eq!(watch_retention_ms(&config), 1_800_000);
    let mut first = WatchRecord {
        candidate: fixtures::candidate(),
        next_due: fixtures::timestamp(100),
        last_seen: fixtures::timestamp(0),
        order: 1,
    };
    let mut second = first.clone();
    second.candidate.token = fixtures::token(2);
    second.last_seen = fixtures::timestamp(1);
    second.order = 2;
    assert!(watch_order(&first, &second).is_lt());
    first.next_due = fixtures::timestamp(101);
    assert!(watch_order(&first, &second).is_gt());
    let mut watch = BTreeMap::from([
        (first.candidate.token.clone(), first),
        (second.candidate.token.clone(), second),
    ]);
    assert_eq!(expire_watch(&mut watch, fixtures::timestamp(100), 100), 1);
    assert_eq!(watch.len(), 1);
    assert!(watch.contains_key(&fixtures::token(2)));
}
