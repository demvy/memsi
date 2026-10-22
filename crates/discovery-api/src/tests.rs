use super::*;

#[test]
fn discovery_union_uses_the_broadest_threshold_and_validates_limits() {
    let chain = ChainId::Evm { chain_id: 1 };
    let low = NonNegative::new(10.0).unwrap();
    let high = NonNegative::new(20.0).unwrap();
    let query = DiscoveryQuery::union(chain.clone(), &[Some(high), Some(low)], 100).unwrap();
    assert_eq!(query.min_liquidity_usd, Some(low));
    assert!(query.liquidity_filter.is_none());
    assert!(query.market_cap_filter.is_none());
    assert_eq!(query.digest(), query.clone().digest());
    assert_ne!(
        query.digest(),
        DiscoveryQuery::union(chain.clone(), &[Some(low)], 101)
            .unwrap()
            .digest()
    );
    assert_eq!(
        DiscoveryQuery::union(chain.clone(), &[Some(low), None], 100)
            .unwrap()
            .min_liquidity_usd,
        None
    );
    assert!(DiscoveryQuery::union(chain.clone(), &[], 100).is_err());
    for limit in [0, 1001] {
        assert!(DiscoveryQuery::union(chain.clone(), &[None], limit).is_err());
    }
}

#[test]
fn filter_envelopes_choose_broadest_values_and_inclusive_ties() {
    let lower = |value: f64, strict: bool| {
        Some(MetricBounds {
            gt: strict.then_some(value),
            gte: (!strict).then_some(value),
            ..Default::default()
        })
    };
    let upper = |value: f64, strict: bool| {
        Some(MetricBounds {
            lt: strict.then_some(value),
            lte: (!strict).then_some(value),
            ..Default::default()
        })
    };
    let bounds = union_lower_bound(&[lower(20.0, false), lower(10.0, true)]).unwrap();
    assert_eq!(bounds.gt, Some(10.0));
    assert_eq!(bounds.gte, None);
    assert_eq!(
        union_lower_bound(&[lower(10.0, true), lower(10.0, false)])
            .unwrap()
            .gte,
        Some(10.0)
    );
    let bounds = union_upper_bound(&[upper(10.0, false), upper(20.0, true)]).unwrap();
    assert_eq!(bounds.lt, Some(20.0));
    assert_eq!(bounds.lte, None);
    assert_eq!(
        union_upper_bound(&[upper(10.0, true), upper(10.0, false)])
            .unwrap()
            .lte,
        Some(10.0)
    );
    assert_eq!(
        union_lower_bound(&[lower(10.0, false), lower(10.0, true)])
            .unwrap()
            .gte,
        Some(10.0)
    );
    assert_eq!(
        union_upper_bound(&[upper(10.0, false), upper(10.0, true)])
            .unwrap()
            .lte,
        Some(10.0)
    );
    for values in [vec![], vec![None], vec![Some(MetricBounds::default())]] {
        assert!(union_lower_bound(&values).is_none());
        assert!(union_upper_bound(&values).is_none());
    }

    for value in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(union_lower_bound(&[lower(value, true)]).is_none());
        assert!(union_upper_bound(&[upper(value, true)]).is_none());
    }
}
