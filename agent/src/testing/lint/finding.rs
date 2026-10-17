//! Constat de lint : forme, analyse depuis Rhai, conversions (spec : `finding.sdd`).

use crate::testing::{
    result::{LINT_PREFIX, Suppression, WARN_PREFIX},
    vyniltestset::VynilAssertResult,
};
use common::rhaihandler::{Dynamic, Map};

/// Sévérité d'un constat. `Error` fait échouer la ligne de verdict ; `Warn` la laisse passer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warn,
}

/// Objet Kubernetes concerné par un constat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectRef {
    pub api_version: String,
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
}

/// Un constat tel que retourné par le script de la distribution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub rule: String,
    pub severity: Severity,
    pub object: ObjectRef,
    pub path: Option<String>,
    pub message: String,
    pub source: Option<String>,
    pub suppressed: bool,
    pub reason: Option<String>,
    pub origin: Option<String>,
}

/// Résultat de l'analyse : les constats valides, puis une description par élément invalide.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedFindings {
    pub findings: Vec<Finding>,
    pub invalid: Vec<String>,
}

const INVALID_RULE: &str = "lint:invalid-finding";
/// Noms de règle réservés aux lignes que le moteur produit lui-même (`lint:execution`,
/// `lint:invalid-finding`, `lint:sections`).
const RESERVED_RULES: [&str; 3] = ["execution", "invalid-finding", "sections"];

/// Analyse la valeur retournée par le script. Un élément mal formé devient une description
/// dans `invalid` : rien n'est écarté silencieusement.
#[must_use]
pub fn parse_findings(value: &Dynamic) -> ParsedFindings {
    let Ok(items) = value.clone().into_array() else {
        return ParsedFindings {
            findings: Vec::new(),
            invalid: vec![format!("findings: expected an array, got {}", value.type_name())],
        };
    };
    let mut parsed = ParsedFindings::default();
    for (index, item) in items.iter().enumerate() {
        match parse_finding(item) {
            Ok(finding) => parsed.findings.push(finding),
            Err(reason) => parsed.invalid.push(format!("finding #{index}: {reason}")),
        }
    }
    parsed
}

fn parse_finding(item: &Dynamic) -> Result<Finding, String> {
    let map = item
        .clone()
        .try_cast::<Map>()
        .ok_or_else(|| "expected a map".to_string())?;
    let severity = match req_str(&map, "severity")?.as_str() {
        "error" => Severity::Error,
        "warn" => Severity::Warn,
        other => return Err(format!("`severity` must be `error` or `warn`, got `{other}`")),
    };
    let object = match field(&map, "object") {
        None => return Err("`object` is required".to_string()),
        Some(value) => {
            let object = value
                .clone()
                .try_cast::<Map>()
                .ok_or_else(|| "`object` must be a map".to_string())?;
            ObjectRef {
                api_version: req_str(&object, "apiVersion")?,
                kind: req_str(&object, "kind")?,
                namespace: opt_str(&object, "namespace")?.filter(|ns| !ns.is_empty()),
                name: req_str(&object, "name")?,
            }
        }
    };
    let suppressed = match field(&map, "suppressed") {
        None => false,
        Some(value) => value
            .as_bool()
            .map_err(|_| "`suppressed` must be a boolean".to_string())?,
    };
    let reason = opt_str(&map, "reason")?;
    if suppressed && reason.as_deref().is_none_or(str::is_empty) {
        return Err("`reason` is required when `suppressed` is true".to_string());
    }
    let rule = req_str(&map, "rule")?;
    if RESERVED_RULES.contains(&rule.as_str()) {
        return Err(format!("`rule` `{rule}` is reserved for the engine's own rows"));
    }
    Ok(Finding {
        rule,
        severity,
        object,
        path: opt_str(&map, "path")?,
        message: req_str(&map, "message")?,
        source: opt_str(&map, "source")?,
        suppressed,
        reason,
        origin: opt_str(&map, "origin")?,
    })
}

/// Champ présent et non `()` ; `()` est traité comme absent.
fn field<'a>(map: &'a Map, key: &str) -> Option<&'a Dynamic> {
    map.get(key).filter(|v| !v.is_unit())
}

fn opt_str(map: &Map, key: &str) -> Result<Option<String>, String> {
    field(map, key)
        .map(|value| {
            value
                .clone()
                .into_string()
                .map_err(|_| format!("`{key}` must be a string"))
        })
        .transpose()
}

fn req_str(map: &Map, key: &str) -> Result<String, String> {
    match opt_str(map, key)? {
        Some(s) if !s.is_empty() => Ok(s),
        Some(_) => Err(format!("`{key}` must not be empty")),
        None => Err(format!("`{key}` is required")),
    }
}

/// `{kind} {namespace}/{name}`, ou `{kind} {name}` sans namespace.
fn object_label(object: &ObjectRef) -> String {
    object.namespace.as_ref().map_or_else(
        || format!("{} {}", object.kind, object.name),
        |ns| format!("{} {ns}/{}", object.kind, object.name),
    )
}

/// Lignes de verdict : une par constat non supprimé, puis une par élément invalide.
#[must_use]
pub fn to_rows(findings: &[Finding], invalid: &[String]) -> Vec<VynilAssertResult> {
    let mut rows: Vec<VynilAssertResult> = findings.iter().filter(|f| !f.suppressed).map(to_row).collect();
    rows.extend(invalid.iter().map(|reason| VynilAssertResult {
        name: INVALID_RULE.to_string(),
        description: None,
        passed: false,
        message: reason.clone(),
    }));
    rows
}

fn to_row(finding: &Finding) -> VynilAssertResult {
    let suffix = finding
        .path
        .as_ref()
        .map_or_else(String::new, |p| format!(" [{p}]"));
    let (passed, prefix) = match finding.severity {
        Severity::Error => (false, ""),
        Severity::Warn => (true, WARN_PREFIX),
    };
    VynilAssertResult {
        name: format!("{LINT_PREFIX}{}", finding.rule),
        description: Some(object_label(&finding.object)),
        passed,
        message: format!("{prefix}{}{suffix}", finding.message),
    }
}

/// Une suppression par constat marqué `suppressed`, dans l'ordre du script.
#[must_use]
pub fn to_suppressions(test: &str, findings: &[Finding]) -> Vec<Suppression> {
    findings
        .iter()
        .filter(|f| f.suppressed)
        .map(|f| Suppression {
            test: test.to_string(),
            rule: f.rule.clone(),
            kind: f.object.kind.clone(),
            namespace: f.object.namespace.clone(),
            name: f.object.name.clone(),
            path: f.path.clone(),
            reason: f.reason.clone(),
            origin: f.origin.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(src: &str) -> Dynamic {
        rhai::Engine::new().eval::<Dynamic>(src).unwrap()
    }

    #[test]
    fn an_error_finding_becomes_a_failed_row() {
        let parsed = parse_findings(&eval(
            r#"[#{
                rule: "latest-tag", severity: "error",
                object: #{ apiVersion: "apps/v1", kind: "Deployment", namespace: "app", name: "web" },
                path: "spec.template.spec.containers[0].image",
                message: "image uses the latest tag"
            }]"#,
        ));
        let rows = to_rows(&parsed.findings, &parsed.invalid);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "lint:latest-tag");
        assert_eq!(rows[0].description.as_deref(), Some("Deployment app/web"));
        assert!(!rows[0].passed);
        assert!(
            rows[0]
                .message
                .ends_with(" [spec.template.spec.containers[0].image]")
        );
    }

    #[test]
    fn a_warning_passes_with_a_prefix() {
        let parsed = parse_findings(&eval(
            r#"[#{
                rule: "no-limits", severity: "warn",
                object: #{ apiVersion: "apps/v1", kind: "Deployment", name: "web" },
                message: "no resource limits"
            }]"#,
        ));
        let rows = to_rows(&parsed.findings, &parsed.invalid);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].passed);
        assert!(rows[0].message.starts_with("warning: "));
    }

    #[test]
    fn a_suppressed_finding_is_inventoried_and_silent() {
        let parsed = parse_findings(&eval(
            r#"[#{
                rule: "latest-tag", severity: "error",
                object: #{ apiVersion: "apps/v1", kind: "Deployment", namespace: "app", name: "web" },
                message: "image uses the latest tag",
                suppressed: true, reason: "legacy image"
            }]"#,
        ));
        let rows = to_rows(&parsed.findings, &parsed.invalid);
        assert!(rows.is_empty());
        let suppressions = to_suppressions("t1", &parsed.findings);
        assert_eq!(suppressions.len(), 1);
        assert_eq!(suppressions[0].rule, "latest-tag");
        assert_eq!(suppressions[0].kind, "Deployment");
        assert_eq!(suppressions[0].name, "web");
        assert_eq!(suppressions[0].reason.as_deref(), Some("legacy image"));
    }

    #[test]
    fn a_suppression_without_a_reason_is_invalid() {
        let parsed = parse_findings(&eval(
            r#"[
                #{
                    rule: "latest-tag", severity: "error",
                    object: #{ apiVersion: "apps/v1", kind: "Deployment", name: "web" },
                    message: "image uses the latest tag", suppressed: true
                },
                #{
                    rule: "latest-tag", severity: "error",
                    object: #{ apiVersion: "apps/v1", kind: "Deployment", name: "web" },
                    message: "image uses the latest tag", suppressed: true, reason: ""
                }
            ]"#,
        ));
        assert_eq!(parsed.findings.len(), 0);
        let rows = to_rows(&parsed.findings, &parsed.invalid);
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|r| r.name == "lint:invalid-finding" && !r.passed && r.message.contains("`reason`"))
        );
        assert_eq!(to_suppressions("t1", &parsed.findings).len(), 0);
    }

    #[test]
    fn a_malformed_finding_is_never_dropped() {
        let parsed = parse_findings(&eval(r#"[#{ rule: "x" }, 42]"#));
        assert_eq!(parsed.findings.len(), 0);
        let rows = to_rows(&parsed.findings, &parsed.invalid);
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|r| r.name == "lint:invalid-finding" && !r.passed && r.description.is_none())
        );
        assert!(rows[0].message.contains("finding #0"));
        assert!(rows[1].message.contains("finding #1"));
    }

    #[test]
    fn reserved_rule_names_are_invalid_findings() {
        let parsed = parse_findings(&eval(
            r#"[
                #{ rule: "execution", severity: "error", object: #{ apiVersion: "v1", kind: "ConfigMap", name: "web" }, message: "x" },
                #{ rule: "invalid-finding", severity: "error", object: #{ apiVersion: "v1", kind: "ConfigMap", name: "web" }, message: "x" },
                #{ rule: "sections", severity: "error", object: #{ apiVersion: "v1", kind: "ConfigMap", name: "web" }, message: "x" },
                #{ rule: "latest-tag", severity: "error", object: #{ apiVersion: "v1", kind: "ConfigMap", name: "web" }, message: "x" }
            ]"#,
        ));
        assert_eq!(parsed.findings.len(), 1);
        assert_eq!(parsed.findings[0].rule, "latest-tag");
        assert_eq!(parsed.invalid.len(), 3, "{:?}", parsed.invalid);
        for (index, rule) in ["execution", "invalid-finding", "sections"].iter().enumerate() {
            let expected = format!("finding #{index}: ");
            assert!(
                parsed.invalid[index].starts_with(&expected),
                "{}",
                parsed.invalid[index]
            );
            assert!(parsed.invalid[index].contains(rule), "{}", parsed.invalid[index]);
        }
    }

    #[test]
    fn an_empty_namespace_is_no_namespace() {
        let parsed = parse_findings(&eval(
            r#"[#{
                rule: "latest-tag", severity: "error",
                object: #{ apiVersion: "v1", kind: "ConfigMap", namespace: "", name: "web" },
                message: "x"
            }]"#,
        ));
        assert!(parsed.invalid.is_empty(), "{:?}", parsed.invalid);
        assert_eq!(parsed.findings[0].object.namespace, None);
        let rows = to_rows(&parsed.findings, &parsed.invalid);
        assert_eq!(rows[0].description.as_deref(), Some("ConfigMap web"));
    }

    #[test]
    fn invalid_finding_messages_are_in_english() {
        let parsed = parse_findings(&eval(
            r#"[
                #{ rule: "sections", severity: "error", object: #{ apiVersion: "v1", kind: "ConfigMap", name: "web" }, message: "x" },
                #{ rule: "latest-tag", severity: "error", object: #{ apiVersion: "v1", kind: "ConfigMap", name: "web" } },
                #{ rule: "latest-tag", severity: "fatal", object: #{ apiVersion: "v1", kind: "ConfigMap", name: "web" }, message: "x" }
            ]"#,
        ));
        assert_eq!(parsed.findings.len(), 0);
        assert_eq!(parsed.invalid.len(), 3, "{:?}", parsed.invalid);
        assert!(
            parsed.invalid[0].starts_with("finding #0: "),
            "{}",
            parsed.invalid[0]
        );
        assert!(parsed.invalid[0].contains("reserved"), "{}", parsed.invalid[0]);
        assert!(
            parsed.invalid[1].starts_with("finding #1: "),
            "{}",
            parsed.invalid[1]
        );
        assert!(
            parsed.invalid[1].contains("`message` is required"),
            "{}",
            parsed.invalid[1]
        );
        assert!(
            parsed.invalid[2].starts_with("finding #2: "),
            "{}",
            parsed.invalid[2]
        );
        assert!(
            parsed.invalid[2].contains("`severity` must be"),
            "{}",
            parsed.invalid[2]
        );
    }

    #[test]
    fn cluster_scoped_object_has_no_namespace_in_its_description() {
        let parsed = parse_findings(&eval(
            r#"[#{
                rule: "no-wildcard", severity: "error",
                object: #{ apiVersion: "rbac.authorization.k8s.io/v1", kind: "ClusterRole", name: "admin" },
                message: "wildcard verb"
            }]"#,
        ));
        let rows = to_rows(&parsed.findings, &parsed.invalid);
        assert_eq!(rows[0].description.as_deref(), Some("ClusterRole admin"));
    }
}
