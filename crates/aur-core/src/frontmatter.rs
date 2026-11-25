//! Frontmatter extraction and serialisation.
//!
//! We deliberately do not use a heavyweight YAML frontmatter crate: the
//! grammar is small, and keeping it local lets us preserve unknown keys
//! instead of silently dropping a field a user added by hand.

use serde_yaml::{Mapping, Value};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frontmatter {
    pub present: bool,
    pub id: Option<String>,
    pub kind: Option<String>,
    pub date: Option<String>,
    pub project: Option<String>,
    pub status: Option<String>,
    pub tags: Vec<String>,
    pub title: Option<String>,
    /// Unknown keys, preserved verbatim on write.
    pub extra: Vec<(String, String)>,
}

fn clean_scalar(s: &str) -> String {
    let t = s.trim();
    let t = t.trim_matches('"').trim_matches('\'');
    t.to_string()
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => String::new(),
        Value::Sequence(items) => items
            .iter()
            .map(value_to_string)
            .collect::<Vec<_>>()
            .join(", "),
        other => serde_yaml::to_string(other)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}

impl Frontmatter {
    /// Parse a YAML frontmatter block (without the `---` fences).
    pub fn from_yaml(yaml: &str) -> Self {
        let mut fm = Frontmatter {
            present: true,
            ..Default::default()
        };
        let Ok(Value::Mapping(map)) = serde_yaml::from_str::<Value>(yaml) else {
            return fm;
        };
        for (k, v) in map {
            let key = value_to_string(&k);
            match key.as_str() {
                "id" => fm.id = non_empty(value_to_string(&v)),
                "kind" => fm.kind = non_empty(value_to_string(&v)),
                "date" => fm.date = non_empty(value_to_string(&v)),
                "project" => fm.project = non_empty(value_to_string(&v)),
                "status" => fm.status = non_empty(value_to_string(&v)),
                "title" => fm.title = non_empty(value_to_string(&v)),
                "tags" => {
                    fm.tags = match &v {
                        Value::Sequence(items) => items
                            .iter()
                            .map(value_to_string)
                            .map(|s| clean_scalar(&s))
                            .filter(|s| !s.is_empty())
                            .collect(),
                        Value::String(s) => s
                            .split(',')
                            .map(clean_scalar)
                            .filter(|p| !p.is_empty())
                            .collect(),
                        _ => Vec::new(),
                    }
                }
                _ => fm.extra.push((key, value_to_string(&v))),
            }
        }
        fm
    }

    /// Serialise to a YAML block, stable key order, unknown keys last.
    pub fn to_yaml(&self) -> String {
        let mut map = Mapping::new();
        let mut put = |k: &str, v: &str| {
            map.insert(Value::String(k.to_string()), Value::String(v.to_string()));
        };
        if let Some(id) = &self.id {
            put("id", id);
        }
        if let Some(kind) = &self.kind {
            put("kind", kind);
        }
        if let Some(date) = &self.date {
            put("date", date);
        }
        if let Some(project) = &self.project {
            put("project", project);
        }
        if let Some(status) = &self.status {
            put("status", status);
        }
        if let Some(title) = &self.title {
            put("title", title);
        }
        if !self.tags.is_empty() {
            map.insert(
                Value::String("tags".into()),
                Value::Sequence(self.tags.iter().map(|t| Value::String(t.clone())).collect()),
            );
        }
        for (k, v) in &self.extra {
            if map.contains_key(Value::String(k.clone())) {
                continue;
            }
            map.insert(Value::String(k.clone()), Value::String(v.clone()));
        }
        serde_yaml::to_string(&map).unwrap_or_default()
    }
}

fn non_empty(s: String) -> Option<String> {
    let t = s.trim().to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

/// Split a raw markdown file into (frontmatter yaml, body).
///
/// A frontmatter block only counts when it is the very first thing in the
/// file and is closed by a fence; otherwise the document is treated as plain
/// markdown so we never swallow the start of a real file.
pub fn split_frontmatter(raw: &str) -> (Option<String>, &str) {
    let text = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return (None, text);
    };
    let first_trimmed = first.trim_end_matches(['\r', '\n']);
    if first_trimmed.trim() != "---" {
        return (None, text);
    }

    let mut offset = first.len();
    let mut yaml = String::new();
    for line in lines {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.trim() == "---" || trimmed.trim() == "..." {
            let body_start = offset + line.len();
            return (Some(yaml), &text[body_start.min(text.len())..]);
        }
        yaml.push_str(line);
        offset += line.len();
    }
    // Unterminated fence: not frontmatter.
    (None, text)
}

/// Parse frontmatter and body in one call.
pub fn parse(raw: &str) -> (Frontmatter, &str) {
    match split_frontmatter(raw) {
        (Some(yaml), body) => (Frontmatter::from_yaml(&yaml), body),
        (None, body) => (Frontmatter::default(), body),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\nid: exp-001\nkind: 实验卡\ndate: 2026-11-25\ntags: [a, \"b\"]\nowner: zhou\n---\n## 问题\nhi\n";

    #[test]
    fn parses_fields_and_keeps_unknown_keys() {
        let (fm, body) = parse(SAMPLE);
        assert_eq!(fm.id.as_deref(), Some("exp-001"));
        assert_eq!(fm.kind.as_deref(), Some("实验卡"));
        assert_eq!(fm.tags, vec!["a", "b"]);
        assert_eq!(fm.extra.len(), 1);
        assert!(body.starts_with("## 问题"));
    }

    #[test]
    fn missing_fence_is_not_frontmatter() {
        let raw = "---\nid: x\n\n# Hello";
        let (fm, body) = parse(raw);
        assert!(!fm.present);
        assert!(body.contains("# Hello"));
    }

    #[test]
    fn roundtrip_keeps_unknown() {
        let (fm, _) = parse(SAMPLE);
        let yaml = fm.to_yaml();
        assert!(yaml.contains("owner: zhou"), "{yaml}");
        assert!(yaml.contains("实验卡"));
    }
}
