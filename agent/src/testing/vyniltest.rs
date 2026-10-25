use super::vyniltestset::{
    VynilAssert, VynilAssertMatch, VynilAssertResult, VynilAssertSelector, VynilTestSetMocks,
};
use common::rhaihandler::Dynamic;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[must_use]
pub fn appslug(pkg: &str, inst: &str) -> String {
    if pkg == inst {
        inst.to_string()
    } else {
        let slug = format!("{inst}-{pkg}");
        if slug.chars().count() > 28 {
            slug.chars().take(28).collect()
        } else {
            slug
        }
    }
}

/// Vynil Test
#[allow(non_snake_case)]
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct VynilTest {
    pub apiVersion: String,
    pub kind: String,
    /// Metadata for a test
    pub metadata: VynilTestMeta,
    /// Target instance to test against
    pub instance: VynilTestInstance,
    /// `TestSet` references with variable overrides
    pub testSets: Option<Vec<VynilTestSetRef>>,
    /// Additional mocks at test level
    pub mocks: Option<VynilTestSetMocks>,
    /// Assert definitions
    pub asserts: Option<Vec<VynilAssert>>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct VynilTestMeta {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct VynilTestInstance {
    pub name: String,
    pub namespace: String,
    pub options: Option<BTreeMap<String, serde_json::Value>>,
    /// Node names mocked for the run, so `ctx::run` derives `cluster.ha` from them.
    pub nodes: Option<Vec<String>>,
    /// Sub-directory of the package `tests/` used as the run's configuration directory.
    pub config_dir: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct VynilTestSetRef {
    pub testSet: String,
    pub variables: Option<BTreeMap<String, serde_json::Value>>,
}

// ── Assert helpers (private) ────────────────────────────────────────────────

/// Checks whether `expected` is a subset of `actual`:
/// every key/value in expected must exist and match in actual.
/// Arrays: every element in expected must have at least one match in actual (unordered subset).
fn json_subset_match(expected: &serde_json::Value, actual: &serde_json::Value) -> bool {
    match (expected, actual) {
        (serde_json::Value::Object(exp), serde_json::Value::Object(act)) => exp
            .iter()
            .all(|(k, v)| act.get(k).is_some_and(|av| json_subset_match(v, av))),
        (serde_json::Value::Array(exp), serde_json::Value::Array(act)) => {
            exp.iter().all(|e| act.iter().any(|a| json_subset_match(e, a)))
        }
        _ => expected == actual,
    }
}

/// Returns human-readable differences between `expected` (subset) and `actual`.
fn json_subset_diff(expected: &serde_json::Value, actual: &serde_json::Value, path: &str) -> Vec<String> {
    match (expected, actual) {
        (serde_json::Value::Object(exp), serde_json::Value::Object(act)) => exp
            .iter()
            .flat_map(|(k, v)| {
                let child = if path.is_empty() {
                    format!(".{k}")
                } else {
                    format!("{path}.{k}")
                };
                act.get(k).map_or_else(
                    || vec![format!("{child}: expected {v}, got <missing>")],
                    |av| json_subset_diff(v, av, &child),
                )
            })
            .collect(),
        (serde_json::Value::Array(exp), serde_json::Value::Array(act)) => exp
            .iter()
            .enumerate()
            .filter(|(_, e)| !act.iter().any(|a| json_subset_match(e, a)))
            .map(|(i, e)| format!("{path}[{i}]: no match found for {e}"))
            .collect(),
        _ => {
            if expected == actual {
                vec![]
            } else {
                vec![format!("{path}: expected {expected}, got {actual}")]
            }
        }
    }
}

fn object_label(d: &Dynamic) -> String {
    let Ok(map) = d.as_map_ref() else {
        return "<unknown>".into();
    };
    let kind = map
        .get("kind")
        .and_then(|v| v.clone().into_string().ok())
        .unwrap_or_default();
    let (name, ns) = map
        .get("metadata")
        .and_then(|m| m.as_map_ref().ok())
        .map(|meta| {
            let name = meta
                .get("name")
                .and_then(|v| v.clone().into_string().ok())
                .unwrap_or_default();
            let ns = meta
                .get("namespace")
                .and_then(|v| v.clone().into_string().ok())
                .unwrap_or_default();
            (name, ns)
        })
        .unwrap_or_default();
    if ns.is_empty() {
        format!("{kind}/{name}")
    } else {
        format!("{kind}/{ns}/{name}")
    }
}

/// Returns true if a generated Dynamic object matches the selector criteria.
fn matches_selector(d: &Dynamic, selector: &VynilAssertSelector) -> bool {
    let Ok(map) = d.as_map_ref() else { return false };
    if let Some(kind) = &selector.kind {
        let actual = map.get("kind").and_then(|v| v.clone().into_string().ok());
        if actual.as_deref() != Some(kind) {
            return false;
        }
    }
    if selector.name.is_none() && selector.namespace.is_none() {
        return true;
    }
    let Some(meta_dyn) = map.get("metadata") else {
        return false;
    };
    let Ok(meta) = meta_dyn.as_map_ref() else {
        return false;
    };
    if let Some(name) = &selector.name {
        let actual = meta.get("name").and_then(|v| v.clone().into_string().ok());
        if actual.as_deref() != Some(name) {
            return false;
        }
    }
    if let Some(ns) = &selector.namespace {
        let actual = meta.get("namespace").and_then(|v| v.clone().into_string().ok());
        if actual.as_deref() != Some(ns) {
            return false;
        }
    }
    true
}

/// Builds the "did you mean" hint when a selector matched nothing but similar objects exist.
fn no_match_hint(generated: &[Dynamic], selector: &VynilAssertSelector) -> String {
    let hints: Vec<String> = generated
        .iter()
        .filter(|d| {
            let Ok(map) = d.as_map_ref() else { return false };
            let kind_match = selector.kind.as_ref().is_some_and(|k| {
                map.get("kind")
                    .and_then(|v| v.clone().into_string().ok())
                    .as_deref()
                    .is_some_and(|ak| ak.eq_ignore_ascii_case(k))
            });
            let name_match = selector.name.as_ref().is_some_and(|n| {
                map.get("metadata")
                    .and_then(|m| m.as_map_ref().ok())
                    .and_then(|meta| meta.get("name").and_then(|v| v.clone().into_string().ok()))
                    .as_deref()
                    .is_some_and(|an| an.eq_ignore_ascii_case(n))
            });
            kind_match || name_match
        })
        .map(object_label)
        .collect();
    if hints.is_empty() {
        "no objects matched selector".into()
    } else {
        format!("no objects matched selector (did you mean: {})", hints.join(", "))
    }
}

/// Renders detail lines for the given object indices: per-object JSON diffs when
/// `show_diff` is true, plain "(matched)" labels otherwise.
fn detail_lines(
    indices: &[usize],
    objects: &[&Dynamic],
    jsons: &[serde_json::Value],
    expected: &serde_json::Value,
    show_diff: bool,
) -> String {
    if indices.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = indices
        .iter()
        .map(|&i| {
            let label = objects
                .get(i)
                .map_or_else(|| "<unknown>".into(), |d| object_label(d));
            if !show_diff {
                return format!("  {label} (matched)");
            }
            let diffs = jsons
                .get(i)
                .map_or_else(Vec::new, |json| json_subset_diff(expected, json, ""));
            if diffs.is_empty() {
                format!("  {label}")
            } else {
                format!("  {label}: {}", diffs.join(", "))
            }
        })
        .collect();
    format!("\n{}", lines.join("\n"))
}

/// Evaluates a value-carrying matcher against the number of matching selected objects.
fn value_verdict(
    matcher: &VynilAssertMatch,
    matching: usize,
    total: usize,
    non_match_detail: &dyn Fn() -> String,
    match_detail: &dyn Fn() -> String,
) -> (bool, String) {
    match matcher {
        VynilAssertMatch::All => {
            if matching == total {
                (true, format!("{matching}/{total} match"))
            } else {
                (
                    false,
                    format!("{matching}/{total} match, expected all{}", non_match_detail()),
                )
            }
        }
        VynilAssertMatch::Any => {
            if matching > 0 {
                (true, format!("{matching}/{total} match"))
            } else {
                (
                    false,
                    format!("0/{total} match, expected at least one{}", non_match_detail()),
                )
            }
        }
        VynilAssertMatch::Exact(n) => {
            let n = usize::try_from(*n).unwrap_or(usize::MAX);
            if matching == n && total == n {
                (true, format!("{matching}/{total} match"))
            } else {
                (
                    false,
                    format!(
                        "{matching}/{total} match, expected exactly {n}{}",
                        non_match_detail()
                    ),
                )
            }
        }
        VynilAssertMatch::AtLeast(n) => {
            let n = usize::try_from(*n).unwrap_or(usize::MAX);
            if matching >= n {
                (true, format!("{matching}/{total} match"))
            } else {
                (
                    false,
                    format!(
                        "{matching}/{total} match, expected at least {n}{}",
                        non_match_detail()
                    ),
                )
            }
        }
        VynilAssertMatch::AtMost(n) => {
            let n = usize::try_from(*n).unwrap_or(usize::MAX);
            if matching <= n {
                (true, format!("{matching}/{total} match"))
            } else {
                (
                    false,
                    format!("{matching}/{total} match, expected at most {n}{}", match_detail()),
                )
            }
        }
        VynilAssertMatch::None => {
            if matching == 0 {
                (true, format!("0/{total} match as expected"))
            } else {
                (
                    false,
                    format!("{matching}/{total} match, expected none{}", match_detail()),
                )
            }
        }
    }
}

/// Evaluates a bare matcher (no expected value) against the number of selected objects.
fn count_verdict(matcher: &VynilAssertMatch, total: usize) -> (bool, String) {
    match matcher {
        VynilAssertMatch::All | VynilAssertMatch::Any => {
            if total > 0 {
                (true, format!("{total} match"))
            } else {
                (false, format!("{total} match, expected at least one"))
            }
        }
        VynilAssertMatch::Exact(n) => {
            let n = usize::try_from(*n).unwrap_or(usize::MAX);
            if total == n {
                (true, format!("{total} match"))
            } else {
                (false, format!("{total} match, expected exactly {n}"))
            }
        }
        VynilAssertMatch::AtLeast(n) => {
            let n = usize::try_from(*n).unwrap_or(usize::MAX);
            if total >= n {
                (true, format!("{total} match"))
            } else {
                (false, format!("{total} match, expected at least {n}"))
            }
        }
        VynilAssertMatch::AtMost(n) => {
            let n = usize::try_from(*n).unwrap_or(usize::MAX);
            if total <= n {
                (true, format!("{total} match"))
            } else {
                (false, format!("{total} match, expected at most {n}"))
            }
        }
        VynilAssertMatch::None => {
            if total == 0 {
                (true, format!("{total} match as expected"))
            } else {
                (false, format!("{total} match, expected none"))
            }
        }
    }
}

impl VynilTest {
    /// Runs all asserts against the generated objects and returns results.
    #[must_use]
    pub fn run_asserts(&self, generated: &[Dynamic]) -> Vec<VynilAssertResult> {
        let Some(asserts) = &self.asserts else {
            return vec![];
        };
        asserts
            .iter()
            .map(|a| {
                let selected: Vec<&Dynamic> = generated
                    .iter()
                    .filter(|d| matches_selector(d, &a.selector))
                    .collect();
                let total = selected.len();
                if total == 0 && !matches!(a.matcher, VynilAssertMatch::None | VynilAssertMatch::AtMost(_)) {
                    return VynilAssertResult {
                        name: a.name.clone(),
                        description: a.description.clone(),
                        passed: false,
                        message: no_match_hint(generated, &a.selector),
                    };
                }
                let (passed, message) = a.value.clone().map_or_else(
                    || count_verdict(&a.matcher, total),
                    |value| {
                        let selected_jsons: Vec<serde_json::Value> = selected
                            .iter()
                            .map(|d| {
                                serde_json::from_str(&serde_json::to_string(*d).unwrap_or_default())
                                    .unwrap_or_default()
                            })
                            .collect();
                        let (matched_idx, non_matched_idx): (Vec<usize>, Vec<usize>) =
                            (0..total).partition(|&i| {
                                selected_jsons
                                    .get(i)
                                    .is_some_and(|json| json_subset_match(&value, json))
                            });
                        let matching = matched_idx.len();
                        value_verdict(
                            &a.matcher,
                            matching,
                            total,
                            &|| detail_lines(&non_matched_idx, &selected, &selected_jsons, &value, true),
                            &|| detail_lines(&matched_idx, &selected, &selected_jsons, &value, false),
                        )
                    },
                );
                VynilAssertResult {
                    name: a.name.clone(),
                    description: a.description.clone(),
                    passed,
                    message,
                }
            })
            .collect()
    }
}
