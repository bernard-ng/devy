//! Reading fields out of payloads that octocrab leaves as raw JSON.

use serde_json::Value;

/// Follows a dotted path (`"workflow_run.actor.login"`) through objects and arrays of objects.
pub fn at<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(value, |current, key| current.get(key))
}

/// A non-empty string at `path`.
pub fn text<'a>(value: &'a Value, path: &str) -> Option<&'a str> {
    at(value, path)?.as_str().filter(|s| !s.is_empty())
}

pub fn number(value: &Value, path: &str) -> Option<u64> {
    at(value, path)?.as_u64()
}

pub fn flag(value: &Value, path: &str) -> bool {
    at(value, path).and_then(Value::as_bool).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nested_fields_and_ignores_missing_or_empty_ones() {
        let v = serde_json::json!({ "a": { "b": "x", "n": 3, "t": true, "e": "" } });
        assert_eq!(text(&v, "a.b"), Some("x"));
        assert_eq!(text(&v, "a.e"), None);
        assert_eq!(text(&v, "a.missing.deeper"), None);
        assert_eq!(number(&v, "a.n"), Some(3));
        assert!(flag(&v, "a.t"));
        assert!(!flag(&v, "a.nope"));
    }
}
