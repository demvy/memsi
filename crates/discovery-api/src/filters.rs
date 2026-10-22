use domain::MetricBounds;
/// Broad OR-envelope: the smallest lower bound; inclusive wins equal boundaries.
/// An unbounded profile disables pushdown for this metric. Upper bounds stay local.
pub fn union_lower_bound(profiles: &[Option<MetricBounds>]) -> Option<MetricBounds> {
    let mut lower = Vec::new();
    for profile in profiles {
        let bounds = profile.as_ref()?;
        let boundary = bounds
            .gt
            .map(|v| (v, true))
            .or_else(|| bounds.gte.map(|v| (v, false)))?;
        if !boundary.0.is_finite() || boundary.0 < 0.0 {
            return None;
        }

        lower.push(boundary);
    }

    let (value, strict) = lower
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))?;
    Some(MetricBounds {
        gt: strict.then_some(value),
        gte: (!strict).then_some(value),
        ..Default::default()
    })
}

/// Broad OR-envelope for upper bounds: largest ceiling; inclusive wins ties.
pub fn union_upper_bound(profiles: &[Option<MetricBounds>]) -> Option<MetricBounds> {
    let mut upper = Vec::new();
    for profile in profiles {
        let bounds = profile.as_ref()?;
        let boundary = bounds
            .lt
            .map(|v| (v, true))
            .or_else(|| bounds.lte.map(|v| (v, false)))?;
        if !boundary.0.is_finite() || boundary.0 < 0.0 {
            return None;
        }

        upper.push(boundary);
    }

    let (value, strict) = upper
        .into_iter()
        .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)))?;
    Some(MetricBounds {
        lt: strict.then_some(value),
        lte: (!strict).then_some(value),
        ..Default::default()
    })
}
