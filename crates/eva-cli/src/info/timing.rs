use eva_mff::Event;
use serde_json::{json, Value};
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Matching {
    Nearest,
    Next,
    Previous,
    Ordinal,
}
impl Matching {
    pub fn name(self) -> &'static str {
        match self {
            Self::Nearest => "nearest",
            Self::Next => "next",
            Self::Previous => "previous",
            Self::Ordinal => "ordinal",
        }
    }
}
pub fn stats(values: &[f64]) -> Value {
    if values.is_empty() {
        return json!({"count":0,"mean_ms":null,"median_ms":null,"std_ms":null,"min_ms":null,"max_ms":null});
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = values.len();
    let mean = values.iter().sum::<f64>() / n as f64;
    let median = if n.is_multiple_of(2) {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.
    } else {
        sorted[n / 2]
    };
    let sd = (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n as f64).sqrt();
    json!({"count":n,"mean_ms":mean,"median_ms":median,"std_ms":sd,"min_ms":sorted[0],"max_ms":sorted[n-1]})
}
pub fn compare(
    events: &[Event],
    a: &str,
    b: &str,
    method: Matching,
    tolerance_ms: f64,
    details: bool,
) -> Value {
    let source: Vec<_> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| e.code == a)
        .collect();
    let target: Vec<_> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| e.code == b)
        .collect();
    let mut delays = vec![];
    let mut pairs = vec![];
    let mut used = std::collections::BTreeSet::new();
    for (ordinal, (event_index, event)) in source.iter().enumerate() {
        let t = event.onset_ns;
        let i = target.partition_point(|(_, e)| e.onset_ns < t);
        let candidate = match method {
            Matching::Ordinal => (ordinal < target.len()).then_some(ordinal),
            Matching::Next => (i < target.len()).then_some(i),
            Matching::Previous => {
                let j = target.partition_point(|(_, e)| e.onset_ns <= t);
                j.checked_sub(1)
            }
            Matching::Nearest => match (i.checked_sub(1), (i < target.len()).then_some(i)) {
                (Some(left), Some(right)) => Some(
                    if t.abs_diff(target[left].1.onset_ns) <= t.abs_diff(target[right].1.onset_ns) {
                        left
                    } else {
                        right
                    },
                ),
                (left, right) => left.or(right),
            },
        };
        let matched = candidate
            .and_then(|j| target[j].1.onset_ns.checked_sub(t).map(|delta| (j, delta)))
            .filter(|(_, delta)| (*delta as f64 / 1e6).abs() <= tolerance_ms);
        if let Some((j, delta)) = matched {
            delays.push(delta as f64 / 1e6);
            used.insert(j);
        }
        if details {
            pairs.push(json!({"a_event_index":event_index+1,"a_ordinal":ordinal+1,"a_onset_ns":t.to_string(),"a_onset_seconds":t as f64/1e9,"b_event_index":matched.map(|(j,_)|target[j].0+1),"b_ordinal":matched.map(|(j,_)|j+1),"b_onset_ns":matched.map(|(j,_)|target[j].1.onset_ns.to_string()),"b_onset_seconds":matched.map(|(j,_)|target[j].1.onset_ns as f64/1e9),"delay_ms":matched.map(|(_,delta)|delta as f64/1e6)}));
        }
    }
    let mut result = stats(&delays);
    let obj = result.as_object_mut().unwrap();
    obj.insert("a".into(), json!(a));
    obj.insert("b".into(), json!(b));
    obj.insert("matching".into(), json!(method.name()));
    obj.insert("max_lag_ms".into(), json!(tolerance_ms));
    obj.insert("a_count".into(), json!(source.len()));
    obj.insert("b_count".into(), json!(target.len()));
    obj.insert("matched".into(), json!(delays.len()));
    obj.insert("unmatched_a".into(), json!(source.len() - delays.len()));
    obj.insert("unused_b".into(), json!(target.len() - used.len()));
    obj.insert("reused_matches".into(), json!(delays.len() - used.len()));
    if details {
        obj.insert("pairs".into(), json!(pairs));
    }
    result
}
pub fn intervals(events: &[Event], code: &str) -> Value {
    let events: Vec<_> = events.iter().filter(|e| e.code == code).collect();
    let diffs: Vec<_> = events
        .windows(2)
        .filter_map(|v| v[1].onset_ns.checked_sub(v[0].onset_ns))
        .map(|v| v as f64 / 1e6)
        .collect();
    let mut result = stats(&diffs);
    result["code"] = json!(code);
    result["events"] = json!(events.len());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn events(items: &[(&str, i128)]) -> Vec<Event> {
        let mut out: Vec<_> = items
            .iter()
            .map(|(code, ns)| Event {
                code: code.to_string(),
                onset_ns: *ns,
                label: None,
                duration_ns: None,
                raw_begin_time: String::new(),
                source_file: String::new(),
            })
            .collect();
        out.sort_by_key(|e| e.onset_ns);
        out
    }
    #[test]
    fn signed_ties_reuse_and_unmatched_are_explicit() {
        let e = events(&[
            ("B", 0),
            ("A", 10_000_000),
            ("A", 11_000_000),
            ("B", 20_000_000),
            ("A", 21_000_000),
            ("A", 500_000_000),
        ]);
        let v = compare(&e, "A", "B", Matching::Nearest, 10., true);
        assert_eq!(v["matched"], 3);
        assert_eq!(v["unmatched_a"], 1);
        assert_eq!(v["reused_matches"], 1);
        assert_eq!(v["pairs"][0]["delay_ms"], -10.);
        assert_eq!(v["pairs"][1]["delay_ms"], 9.);
        assert!(v["pairs"][3]["delay_ms"].is_null());
        assert_eq!(
            compare(&e, "A", "B", Matching::Next, 10., false)["matched"],
            2
        );
        assert_eq!(
            compare(&e, "A", "B", Matching::Previous, 10., false)["matched"],
            2
        );
        assert_eq!(
            compare(&e, "A", "missing", Matching::Nearest, 10., false)["mean_ms"],
            Value::Null
        );
    }
    #[test]
    fn ordinal_does_not_shift_and_timestamps_keep_nanosecond_precision() {
        let base = 1_000_000_000_000_000_000i128;
        let e = events(&[
            ("A", base),
            ("B", base + 1),
            ("A", base + 1_000_000_000),
            ("B", base + 2_000_000_000),
        ]);
        let v = compare(&e, "A", "B", Matching::Ordinal, 0.001, true);
        assert_eq!(v["matched"], 1);
        assert_eq!(v["pairs"][0]["delay_ms"], 0.000001);
        assert_eq!(intervals(&e, "A")["mean_ms"], 1000.);
        let e = events(&[("A", 0), ("A", 0)]);
        assert_eq!(intervals(&e, "A")["mean_ms"], 0.);
    }
}
