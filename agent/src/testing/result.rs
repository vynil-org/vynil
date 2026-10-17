use super::vyniltestset::VynilAssertResult;
use junit_report::{Duration, ReportBuilder, TestCase, TestSuiteBuilder};
use serde::Serialize;

/// Préfixe du nom d'une ligne du lint (`lint:{règle}`), contrat de `./lint/finding.sdd`.
pub(crate) const LINT_PREFIX: &str = "lint:";
/// Préfixe du message d'un constat `warn` passant, contrat de `./lint/finding.sdd`.
pub(crate) const WARN_PREFIX: &str = "warning: ";

#[derive(Clone, Debug, Serialize)]
pub struct TestResult {
    pub test_name: String,
    pub asserts: Vec<VynilAssertResult>,
    #[serde(serialize_with = "serialize_duration")]
    pub duration: std::time::Duration,
}

fn serialize_duration<S>(d: &std::time::Duration, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_f64(d.as_secs_f64())
}

/// Un contrôle volontairement écarté par une règle de lint, consigné pour relecture.
#[derive(Clone, Debug, Serialize)]
pub struct Suppression {
    pub test: String,
    pub rule: String,
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
    pub path: Option<String>,
    pub reason: Option<String>,
    pub origin: Option<String>,
}

impl Suppression {
    /// `{kind} {namespace}/{name}`, ou `{kind} {name}` pour un objet sans namespace.
    fn object_label(&self) -> String {
        self.namespace.as_ref().map_or_else(
            || format!("{} {}", self.kind, self.name),
            |ns| format!("{} {ns}/{}", self.kind, self.name),
        )
    }

    /// Détail d'une suppression (règle, objet, `raison`), partagé par le texte et `JUnit`.
    fn detail(&self) -> String {
        format!(
            "{} {}: {}",
            self.rule,
            self.object_label(),
            self.reason.as_deref().unwrap_or("(no reason)")
        )
    }
}

#[derive(Clone, Debug)]
pub struct TestResultCollector {
    results: Vec<TestResult>,
    suppressions: Vec<Suppression>,
}

impl TestResultCollector {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            results: Vec::new(),
            suppressions: Vec::new(),
        }
    }

    pub fn add(&mut self, test_name: String, asserts: Vec<VynilAssertResult>, duration: std::time::Duration) {
        self.results.push(TestResult {
            test_name,
            asserts,
            duration,
        });
    }

    pub fn add_suppressions(&mut self, suppressions: Vec<Suppression>) {
        self.suppressions.extend(suppressions);
    }

    #[must_use]
    pub fn suppressions(&self) -> &[Suppression] {
        &self.suppressions
    }

    #[must_use]
    pub const fn total_suppressed(&self) -> usize {
        self.suppressions.len()
    }

    #[must_use]
    pub const fn total_tests(&self) -> usize {
        self.results.len()
    }

    #[must_use]
    pub fn total_asserts(&self) -> usize {
        self.results.iter().map(|r| r.asserts.len()).sum()
    }

    #[must_use]
    pub fn total_passed(&self) -> usize {
        self.results
            .iter()
            .flat_map(|r| &r.asserts)
            .filter(|a| a.passed)
            .count()
    }

    #[must_use]
    pub fn total_failed(&self) -> usize {
        self.results
            .iter()
            .flat_map(|r| &r.asserts)
            .filter(|a| !a.passed)
            .count()
    }

    #[must_use]
    pub fn all_passed(&self) -> bool {
        self.results.iter().all(|r| r.asserts.iter().all(|a| a.passed))
    }

    #[must_use]
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        for result in &self.results {
            let _ = writeln!(
                out,
                "Test: {} ({:.3}s)",
                result.test_name,
                result.duration.as_secs_f64()
            );
            for a in &result.asserts {
                let status = if a.passed { "PASS" } else { "FAIL" };
                match &a.description {
                    Some(desc) if !desc.is_empty() => {
                        let _ = writeln!(out, "  [{status}] {} ({desc}): {}", a.name, a.message);
                    }
                    _ => {
                        let _ = writeln!(out, "  [{status}] {}: {}", a.name, a.message);
                    }
                }
            }
            out.push('\n');
        }
        let _ = writeln!(
            out,
            "Results: {} passed, {} failed, {} total",
            self.total_passed(),
            self.total_failed(),
            self.total_asserts()
        );
        if !self.suppressions.is_empty() {
            let _ = writeln!(out, "Suppressed: {}", self.total_suppressed());
            for s in &self.suppressions {
                let _ = writeln!(out, "  [SKIP] {}: {}", s.test, s.detail());
            }
        }
        out
    }

    #[must_use]
    pub fn to_json(&self) -> String {
        let mut output = serde_json::json!({
            "tests": self.results,
            "summary": {
                "total_tests": self.total_tests(),
                "total_asserts": self.total_asserts(),
                "passed": self.total_passed(),
                "failed": self.total_failed()
            }
        });
        if !self.suppressions.is_empty()
            && let Some(object) = output.as_object_mut()
        {
            object.insert(
                "suppressions".to_string(),
                serde_json::to_value(&self.suppressions).unwrap_or_default(),
            );
        }
        serde_json::to_string_pretty(&output).unwrap_or_default()
    }

    /// # Panics
    ///
    /// Panics if the `JUnit` XML report cannot be written into the in-memory buffer or is not
    /// valid UTF-8 (neither can happen for in-memory `String` test data).
    #[must_use]
    pub fn to_junit(&self) -> String {
        let mut report_builder = ReportBuilder::new();
        for result in &self.results {
            let mut suite = TestSuiteBuilder::new(&result.test_name);
            let nanos = result.duration.as_nanos();
            let assert_count = u128::try_from(result.asserts.len().max(1)).unwrap_or(1);
            let per_assert_nanos = nanos.checked_div(assert_count).unwrap_or(0);
            let secs = i64::try_from(per_assert_nanos / 1_000_000_000).unwrap_or(i64::MAX);
            let nanos_part = i32::try_from(per_assert_nanos % 1_000_000_000).unwrap_or(i32::MAX);
            let tc_dur = Duration::new(secs, nanos_part);
            for a in &result.asserts {
                let mut tc = if a.passed {
                    TestCase::success(&a.name, tc_dur)
                } else {
                    TestCase::failure(&a.name, tc_dur, "assertion", &a.message)
                };
                if let Some(object) = lint_object(a) {
                    tc.set_classname(object);
                }
                suite.add_testcase(tc);
            }
            let skips = self
                .suppressions
                .iter()
                .filter(|s| s.test == result.test_name)
                .map(|s| format!("[SKIP] {}", s.detail()));
            let warns = result.asserts.iter().filter_map(warn_line);
            let lines: Vec<String> = skips.chain(warns).collect();
            if !lines.is_empty() {
                suite.set_system_out(&lines.join("\n"));
            }
            report_builder.add_testsuite(suite.build());
        }
        let report = report_builder.build();
        let mut buf: Vec<u8> = Vec::new();
        if let Err(e) = report.write_xml(&mut buf) {
            panic!("failed to write JUnit XML: {e}");
        }
        match String::from_utf8(buf) {
            Ok(text) => text,
            Err(e) => panic!("JUnit XML is not valid UTF-8: {e}"),
        }
    }
}

/// Description d'objet d'une ligne du lint, si elle en porte une non vide : elle devient le
/// `classname` du cas de test `JUnit`.
fn lint_object(a: &VynilAssertResult) -> Option<&str> {
    if !a.name.starts_with(LINT_PREFIX) {
        return None;
    }
    a.description.as_deref().filter(|d| !d.is_empty())
}

/// Ligne `[WARN]` d'un constat `warn` passant du lint, pour le `<system-out>` de la suite.
fn warn_line(a: &VynilAssertResult) -> Option<String> {
    if !(a.passed && a.name.starts_with(LINT_PREFIX) && a.message.starts_with(WARN_PREFIX)) {
        return None;
    }
    Some(lint_object(a).map_or_else(
        || format!("[WARN] {}: {}", a.name, a.message),
        |object| format!("[WARN] {} ({object}): {}", a.name, a.message),
    ))
}

impl Default for TestResultCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::VynilAssertResult;

    fn passing_assert(name: &str) -> VynilAssertResult {
        VynilAssertResult {
            name: name.to_string(),
            description: None,
            passed: true,
            message: "ok".to_string(),
        }
    }

    fn failing_assert(name: &str) -> VynilAssertResult {
        VynilAssertResult {
            name: name.to_string(),
            description: None,
            passed: false,
            message: "no match".to_string(),
        }
    }

    #[test]
    fn all_passed_empty_collector() {
        let c = TestResultCollector::new();
        assert!(c.all_passed());
    }

    #[test]
    fn all_passed_when_all_asserts_pass() {
        let mut c = TestResultCollector::new();
        c.add(
            "t1".to_string(),
            vec![passing_assert("a"), passing_assert("b")],
            std::time::Duration::ZERO,
        );
        assert!(c.all_passed());
    }

    #[test]
    fn all_passed_false_when_one_assert_fails() {
        let mut c = TestResultCollector::new();
        c.add(
            "t1".to_string(),
            vec![passing_assert("a"), failing_assert("b")],
            std::time::Duration::ZERO,
        );
        assert!(!c.all_passed());
    }

    #[test]
    fn all_passed_false_when_all_asserts_fail() {
        let mut c = TestResultCollector::new();
        c.add(
            "t1".to_string(),
            vec![failing_assert("a"), failing_assert("b")],
            std::time::Duration::ZERO,
        );
        assert!(!c.all_passed());
    }

    #[test]
    fn total_failed_counts_only_failed() {
        let mut c = TestResultCollector::new();
        c.add(
            "t1".to_string(),
            vec![passing_assert("a"), failing_assert("b"), failing_assert("c")],
            std::time::Duration::ZERO,
        );
        assert_eq!(c.total_failed(), 2);
        assert_eq!(c.total_passed(), 1);
    }

    fn suppression(test: &str) -> Suppression {
        Suppression {
            test: test.to_string(),
            rule: "latest-tag".to_string(),
            kind: "Deployment".to_string(),
            namespace: Some("app".to_string()),
            name: "web".to_string(),
            path: None,
            reason: Some("legacy image".to_string()),
            origin: None,
        }
    }

    #[test]
    fn suppressions_do_not_touch_the_verdict() {
        let mut c = TestResultCollector::new();
        c.add(
            "t1".to_string(),
            vec![passing_assert("a"), passing_assert("b")],
            std::time::Duration::ZERO,
        );
        c.add_suppressions(vec![suppression("t1")]);
        assert!(c.all_passed());
        assert_eq!(c.total_passed(), 2);
        assert_eq!(c.total_failed(), 0);
        assert_eq!(c.total_asserts(), 2);
        assert_eq!(c.total_suppressed(), 1);
    }

    /// Strips the `timestamp` attribute, the only field that varies between two renders.
    fn without_timestamp(xml: &str) -> String {
        const KEY: &str = " timestamp=\"";
        let Some(start) = xml.find(KEY) else {
            return xml.to_string();
        };
        let value_start = start + KEY.len();
        let Some(rel_end) = xml[value_start..].find('"') else {
            return xml.to_string();
        };
        format!("{}{}", &xml[..start], &xml[value_start + rel_end + 1..])
    }

    #[test]
    fn forms_unchanged_without_suppressions() {
        let mut c = TestResultCollector::new();
        c.add(
            "t1".to_string(),
            vec![passing_assert("a"), failing_assert("b")],
            std::time::Duration::ZERO,
        );
        assert_eq!(
            c.to_text(),
            "Test: t1 (0.000s)\n  [PASS] a: ok\n  [FAIL] b: no match\n\nResults: 1 passed, 1 failed, 2 total\n"
        );
        assert_eq!(
            c.to_json(),
            concat!(
                "{\n  \"tests\": [\n    {\n      \"test_name\": \"t1\",\n      \"asserts\": [\n        {\n",
                "          \"name\": \"a\",\n          \"description\": null,\n          \"passed\": true,\n",
                "          \"message\": \"ok\"\n        },\n        {\n          \"name\": \"b\",\n",
                "          \"description\": null,\n          \"passed\": false,\n          \"message\": \"no match\"\n",
                "        }\n      ],\n      \"duration\": 0.0\n    }\n  ],\n  \"summary\": {\n    \"total_tests\": 1,\n",
                "    \"total_asserts\": 2,\n    \"passed\": 1,\n    \"failed\": 1\n  }\n}"
            )
        );
        assert!(!c.to_json().contains("suppressions"));
        assert_eq!(
            without_timestamp(&c.to_junit()),
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><testsuites><testsuite id=\"0\" name=\"t1\" \
             package=\"testsuite/t1\" tests=\"2\" errors=\"0\" failures=\"1\" hostname=\"localhost\" \
             time=\"0\"><testcase name=\"a\" time=\"0\"/><testcase name=\"b\" time=\"0\"><failure \
             type=\"assertion\" message=\"no match\"/></testcase></testsuite></testsuites>"
        );
        assert!(!c.to_junit().contains("system-out"));
    }

    #[test]
    fn suppressions_rendered_in_three_forms() {
        let mut c = TestResultCollector::new();
        c.add(
            "t1".to_string(),
            vec![passing_assert("a")],
            std::time::Duration::ZERO,
        );
        c.add_suppressions(vec![suppression("t1")]);

        let text = c.to_text();
        assert!(text.contains("Suppressed: 1\n"));
        assert!(text.contains("  [SKIP] t1: latest-tag Deployment app/web: legacy image\n"));

        let json: serde_json::Value = serde_json::from_str(&c.to_json()).unwrap();
        let entries = json["suppressions"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["test"], "t1");
        assert_eq!(entries[0]["rule"], "latest-tag");
        assert_eq!(entries[0]["kind"], "Deployment");
        assert_eq!(entries[0]["namespace"], "app");
        assert_eq!(entries[0]["name"], "web");
        assert!(entries[0]["path"].is_null());
        assert_eq!(entries[0]["reason"], "legacy image");
        assert!(entries[0]["origin"].is_null());

        let junit = c.to_junit();
        assert_eq!(junit.matches("<system-out>").count(), 1);
        assert!(junit.contains("latest-tag"));
    }

    #[test]
    fn to_text_contains_results_summary() {
        let mut c = TestResultCollector::new();
        c.add(
            "t1".to_string(),
            vec![passing_assert("a"), failing_assert("b")],
            std::time::Duration::ZERO,
        );
        let text = c.to_text();
        assert!(text.contains("Results:"));
        assert!(text.contains("1 passed"));
        assert!(text.contains("1 failed"));
    }

    /// Ligne de verdict d'un constat du lint : nom `lint:{règle}`, description d'objet.
    fn lint_row(name: &str, description: &str, passed: bool, message: &str) -> VynilAssertResult {
        VynilAssertResult {
            name: name.to_string(),
            description: Some(description.to_string()),
            passed,
            message: message.to_string(),
        }
    }

    #[test]
    fn lint_rows_carry_their_object_in_the_junit() {
        let mut c = TestResultCollector::new();
        c.add(
            "t1".to_string(),
            vec![
                lint_row(
                    "lint:latest-tag",
                    "Deployment app/a",
                    false,
                    "image uses the latest tag",
                ),
                lint_row(
                    "lint:latest-tag",
                    "Deployment app/b",
                    false,
                    "image uses the latest tag",
                ),
                lint_row(
                    "lint:latest-tag",
                    "Deployment app/c",
                    false,
                    "image uses the latest tag",
                ),
            ],
            std::time::Duration::ZERO,
        );
        let junit = c.to_junit();
        for object in ["Deployment app/a", "Deployment app/b", "Deployment app/c"] {
            let opening = format!("<testcase name=\"lint:latest-tag\" time=\"0\" classname=\"{object}\">");
            assert!(junit.contains(&opening), "{junit}");
        }
        assert!(!junit.contains("system-out"), "{junit}");
    }

    #[test]
    fn warnings_are_listed_in_the_junit_system_out() {
        let mut c = TestResultCollector::new();
        c.add(
            "t1".to_string(),
            vec![lint_row(
                "lint:no-liveness-probe",
                "Deployment app/web",
                true,
                "warning: no probe",
            )],
            std::time::Duration::ZERO,
        );
        c.add_suppressions(vec![suppression("t1")]);
        let junit = c.to_junit();
        assert!(
            junit.contains(
                "<system-out><![CDATA[[SKIP] latest-tag Deployment app/web: legacy image\n\
                 [WARN] lint:no-liveness-probe (Deployment app/web): warning: no probe]]></system-out>"
            ),
            "{junit}"
        );
    }

    #[test]
    fn ordinary_asserts_keep_their_junit() {
        let mut c = TestResultCollector::new();
        c.add(
            "t1".to_string(),
            vec![
                VynilAssertResult {
                    name: "a".to_string(),
                    description: Some("une description".to_string()),
                    passed: true,
                    message: "ok".to_string(),
                },
                VynilAssertResult {
                    name: "b".to_string(),
                    description: None,
                    passed: true,
                    message: "warning: pas une ligne lint".to_string(),
                },
            ],
            std::time::Duration::ZERO,
        );
        assert_eq!(
            without_timestamp(&c.to_junit()),
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><testsuites><testsuite id=\"0\" name=\"t1\" \
             package=\"testsuite/t1\" tests=\"2\" errors=\"0\" failures=\"0\" hostname=\"localhost\" \
             time=\"0\"><testcase name=\"a\" time=\"0\"/><testcase name=\"b\" time=\"0\"/></testsuite></testsuites>"
        );
    }
}
