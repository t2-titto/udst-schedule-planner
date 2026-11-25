use serde_json::{Map, Value};

fn sorted(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for k in keys {
                out.insert(k.clone(), sorted(&m[k]));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// Canonical JSON text: object keys sorted recursively, no insignificant whitespace.
///
/// The output does not depend on whether `serde_json/preserve_order` is enabled anywhere in the
/// build, so it is safe to hash or sign. (Object insertion order is ignored; array order is kept.)
pub fn canonical_json(v: &Value) -> String {
    // Re-inserting in sorted order makes the output sorted under both map implementations
    // (BTreeMap sorts by itself; IndexMap keeps insertion order).
    serde_json::to_string(&sorted(v)).expect("a serde_json::Value always serialises")
}
