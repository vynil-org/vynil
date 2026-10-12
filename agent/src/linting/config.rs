use crate::linting::LintLevel;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

#[derive(Deserialize, Default, Clone, Debug, Serialize)]
pub struct LintConfig {
    #[serde(default)]
    pub disable: Vec<String>,
    #[serde(default)]
    pub override_: HashMap<String, LintLevel>,
    #[serde(default)]
    pub files: Vec<FileRule>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct FileRule {
    pub glob: String,
    #[serde(default)]
    pub disable: Vec<String>,
    #[serde(default)]
    pub override_: HashMap<String, LintLevel>,
}

impl LintConfig {
    /// Load .vynil-lint.yaml from `package_dir`, return Default if absent.
    ///
    /// A missing, unreadable or invalid config file is not an error: it falls back to the
    /// default lint configuration (with a warning).
    #[must_use]
    pub fn load(package_dir: &Path) -> Self {
        let config_path = package_dir.join(".vynil-lint.yaml");
        if !config_path.exists() {
            return Self::default();
        }

        match std::fs::read_to_string(&config_path) {
            Ok(content) => match serde_yaml::from_str::<Self>(&content) {
                Ok(config) => config,
                Err(e) => {
                    tracing::warn!("Invalid .vynil-lint.yaml: {e}, using defaults");
                    Self::default()
                }
            },
            Err(e) => {
                tracing::warn!("Failed to read .vynil-lint.yaml: {e}, using defaults");
                Self::default()
            }
        }
    }

    /// Resolve effective lint level for a rule on a file.
    /// Priority order (lowest to highest):
    /// 1. Default level
    /// 2. Global override
    /// 3. File glob override (first matching glob)
    /// 4. Inline disable (returns None)
    pub fn resolve_level(
        &self,
        rule: &str,
        file: &Path,
        default_level: LintLevel,
        inline_disabled: &HashSet<String>,
    ) -> Option<LintLevel> {
        // If inline disable contains this rule, suppress it
        if inline_disabled.contains(rule) {
            return None;
        }

        // Check if rule is globally disabled
        if self.disable.contains(&rule.to_string()) {
            return None;
        }

        let mut level = self.override_.get(rule).copied().unwrap_or(default_level);

        // Apply file glob override (first matching)
        for file_rule in &self.files {
            if wildmatch::WildMatch::new(&file_rule.glob).matches(&file.to_string_lossy()) {
                if file_rule.disable.contains(&rule.to_string()) {
                    return None;
                }
                if let Some(&override_level) = file_rule.override_.get(rule) {
                    level = override_level;
                }
                break;
            }
        }

        Some(level)
    }
}

/// Detect a block-mode disable directive (comment alone on line).
/// Returns the text after the keyword, or None if not a block directive.
fn extract_block_disable(trimmed: &str) -> Option<&str> {
    trimmed
        .strip_prefix("// vynil-lint-disable")
        .or_else(|| trimmed.strip_prefix("{{!-- vynil-lint-disable"))
        .or_else(|| trimmed.strip_prefix("# vynil-lint-disable"))
}

/// Detect a block-mode enable directive (comment alone on line).
/// Returns the text after the keyword, or None if not a block directive.
fn extract_block_enable(trimmed: &str) -> Option<&str> {
    trimmed
        .strip_prefix("// vynil-lint-enable")
        .or_else(|| trimmed.strip_prefix("{{!-- vynil-lint-enable"))
        .or_else(|| trimmed.strip_prefix("# vynil-lint-enable"))
}

/// Parse disable comments from source code.
/// Returns a map of rule IDs disabled on each line.
///
/// Block mode (comment alone on its line — applies to all lines until enable):
/// - Rhai: `// vynil-lint-disable rule-a` … `// vynil-lint-enable rule-a`
/// - HBS:  `{{!-- vynil-lint-disable rule-a --}}` … `{{!-- vynil-lint-enable rule-a --}}`
/// - YAML: `# vynil-lint-disable rule-a` … `# vynil-lint-enable rule-a`
///
/// Inline mode (comment after code on the same line — applies to that line only):
/// - Rhai: `code; // vynil-lint-disable rule-a`
/// - HBS:  `{{expr}}{{!-- vynil-lint-disable rule-a --}}`
/// - YAML: `key: val # vynil-lint-disable rule-a`
pub fn parse_inline_disables(source: &str) -> HashMap<usize, HashSet<String>> {
    let mut result: HashMap<usize, HashSet<String>> = HashMap::new();
    let mut active_blocks: HashSet<String> = HashSet::new();

    for (line_num, line) in source.lines().enumerate() {
        let line_number = line_num.saturating_add(1); // 1-based
        let trimmed = line.trim();

        // Block-mode disable: comment alone on this line
        if let Some(rest) = extract_block_disable(trimmed) {
            active_blocks.extend(parse_rules(rest));
            continue;
        }

        // Block-mode enable: comment alone on this line
        if let Some(rest) = extract_block_enable(trimmed) {
            for rule in parse_rules(rest) {
                active_blocks.remove(&rule);
            }
            continue;
        }

        // Propagate active block-disables to this line
        if !active_blocks.is_empty() {
            result
                .entry(line_number)
                .or_default()
                .extend(active_blocks.iter().cloned());
        }

        // Inline mode: comment after code on the same line

        // Rhai: // vynil-lint-disable
        if let Some(pos) = line.find("// vynil-lint-disable") {
            let rest = line.get(pos.saturating_add(21)..).unwrap_or("");
            let rules = parse_rules(rest);
            if !rules.is_empty() {
                result.entry(line_number).or_default().extend(rules);
                continue;
            }
        }

        // HBS: {{!-- vynil-lint-disable
        if let Some(pos) = line.find("{{!-- vynil-lint-disable") {
            let rest = line.get(pos.saturating_add(24)..).unwrap_or("");
            let rules = parse_rules(rest);
            if !rules.is_empty() {
                result.entry(line_number).or_default().extend(rules);
                continue;
            }
        }

        // YAML: # vynil-lint-disable
        if let Some(pos) = line.find("# vynil-lint-disable") {
            let rest = line.get(pos.saturating_add(20)..).unwrap_or("");
            let rules = parse_rules(rest);
            if !rules.is_empty() {
                result.entry(line_number).or_default().extend(rules);
            }
        }
    }

    result
}

/// Parse comma-separated rule IDs from a string.
/// Each comma-separated segment contributes only its first whitespace token as a rule ID.
/// The first segment whose token does not contain '/' is treated as the start of a free-text
/// comment and stops parsing — this allows trailing explanations such as:
///   `rhai/foo, rhai/bar  ← reason why`
fn parse_rules(s: &str) -> HashSet<String> {
    let mut rules = HashSet::new();
    for part in s.split(',') {
        // Take only the first whitespace token; the rest may be a trailing comment
        let raw = part.split_whitespace().next().unwrap_or("");
        // Strip HBS/block-comment closing markers that may appear without a preceding space
        let token = raw.trim_end_matches("--}}").trim_end_matches("*/").trim();
        if token.is_empty() {
            continue;
        }
        if token.contains('/') {
            rules.insert(token.to_string());
        } else {
            // Non-rule token signals start of free-text comment — stop here
            break;
        }
    }
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_returns_default_when_file_absent() {
        let config = LintConfig::load(Path::new("/nonexistent/path"));
        assert_eq!(config.disable, Vec::<String>::new());
        assert!(config.override_.is_empty());
        assert!(config.files.is_empty());
    }

    #[test]
    fn global_disable_suppresses_finding() {
        let mut config = LintConfig::default();
        config.disable.push("hbs/unknown-helper".to_string());

        let inline_disabled = HashSet::new();
        let result = config.resolve_level(
            "hbs/unknown-helper",
            Path::new("test.hbs"),
            LintLevel::Error,
            &inline_disabled,
        );

        assert_eq!(result, None);
    }

    #[test]
    fn global_override_changes_level() {
        let mut config = LintConfig::default();
        config
            .override_
            .insert("hbs/unknown-value".to_string(), LintLevel::Warn);

        let inline_disabled = HashSet::new();
        let result = config.resolve_level(
            "hbs/unknown-value",
            Path::new("test.hbs"),
            LintLevel::Error,
            &inline_disabled,
        );

        assert_eq!(result, Some(LintLevel::Warn));
    }

    #[test]
    fn file_glob_override_takes_priority_over_global() {
        let mut config = LintConfig::default();
        config
            .override_
            .insert("hbs/unknown-partial".to_string(), LintLevel::Error);

        let mut file_rule = FileRule {
            glob: "handlebars/partials/**".to_string(),
            disable: vec![],
            override_: HashMap::new(),
        };
        file_rule
            .override_
            .insert("hbs/unknown-partial".to_string(), LintLevel::Warn);
        config.files.push(file_rule);

        let inline_disabled = HashSet::new();

        // File matching glob → Warn
        let result = config.resolve_level(
            "hbs/unknown-partial",
            Path::new("handlebars/partials/foo.hbs"),
            LintLevel::Error,
            &inline_disabled,
        );
        assert_eq!(result, Some(LintLevel::Warn));

        // File not matching glob → Error (global override)
        let result = config.resolve_level(
            "hbs/unknown-partial",
            Path::new("systems/bar.hbs"),
            LintLevel::Error,
            &inline_disabled,
        );
        assert_eq!(result, Some(LintLevel::Error));
    }

    #[test]
    fn inline_disable_returns_none() {
        let config = LintConfig::default();
        let mut inline_disabled = HashSet::new();
        inline_disabled.insert("test/rule".to_string());

        let result = config.resolve_level(
            "test/rule",
            Path::new("test.txt"),
            LintLevel::Error,
            &inline_disabled,
        );

        assert_eq!(result, None);
    }

    #[test]
    fn parse_rhai_inline_disable() {
        let src = "let x = 1; // vynil-lint-disable rhai/unused-variable\n";
        let map = parse_inline_disables(src);
        assert!(map.contains_key(&1));
        assert!(map[&1].contains("rhai/unused-variable"));
    }

    #[test]
    fn inline_disable_with_trailing_comment() {
        let src = "catch {} // vynil-lint-disable rhai/empty-catch ici c'est paris\n";
        let map = parse_inline_disables(src);
        assert!(map.get(&1).is_some_and(|s| s.contains("rhai/empty-catch")));
    }

    #[test]
    fn inline_disable_multi_rules_with_trailing_comment() {
        let src = "x; // vynil-lint-disable rhai/foo, rhai/bar reason here\n";
        let map = parse_inline_disables(src);
        let set = map.get(&1).expect("line 1 should have disables");
        assert!(set.contains("rhai/foo"));
        assert!(set.contains("rhai/bar"));
    }

    #[test]
    fn inline_disable_comment_with_comma_stops_at_non_rule() {
        let src = "x; // vynil-lint-disable rhai/foo this is, not a rule\n";
        let map = parse_inline_disables(src);
        let set = map.get(&1).expect("line 1 should have disables");
        assert!(set.contains("rhai/foo"));
        assert_eq!(set.len(), 1, "non-rule tokens must not be added as rules");
    }

    #[test]
    fn hbs_inline_disable_with_trailing_comment() {
        let src = "{{foo}}{{!-- vynil-lint-disable hbs/unknown-helper legacy helper --}}\n";
        let map = parse_inline_disables(src);
        assert!(map.get(&1).is_some_and(|s| s.contains("hbs/unknown-helper")));
    }

    #[test]
    fn parse_hbs_inline_disable() {
        let src = "{{foo}}{{!-- vynil-lint-disable hbs/unknown-helper --}}\n";
        let map = parse_inline_disables(src);
        assert!(map.contains_key(&1));
        assert!(map[&1].contains("hbs/unknown-helper"));
    }

    #[test]
    fn parse_yaml_inline_disable() {
        let src = "size: # vynil-lint-disable hbs/unused-option\n";
        let map = parse_inline_disables(src);
        assert!(map.contains_key(&1));
        assert!(map[&1].contains("hbs/unused-option"));
    }

    #[test]
    fn parse_rhai_block_disable() {
        let src = "// vynil-lint-disable rhai/empty-catch\ncode();\ntry {} catch {}\n// vynil-lint-enable rhai/empty-catch\nmore_code();\n";
        let map = parse_inline_disables(src);
        assert!(!map.contains_key(&1), "directive line must not appear in result");
        assert!(map.get(&2).is_some_and(|s| s.contains("rhai/empty-catch")));
        assert!(map.get(&3).is_some_and(|s| s.contains("rhai/empty-catch")));
        assert!(
            !map.contains_key(&4),
            "enable directive line must not appear in result"
        );
        assert!(!map.contains_key(&5), "code after enable must not be disabled");
    }

    #[test]
    fn parse_hbs_block_disable() {
        let src = "{{!-- vynil-lint-disable hbs/unknown-helper --}}\n{{foo}}\n{{!-- vynil-lint-enable hbs/unknown-helper --}}\n{{bar}}\n";
        let map = parse_inline_disables(src);
        assert!(!map.contains_key(&1));
        assert!(map.get(&2).is_some_and(|s| s.contains("hbs/unknown-helper")));
        assert!(!map.contains_key(&3));
        assert!(!map.contains_key(&4));
    }

    #[test]
    fn parse_yaml_block_disable() {
        let src =
            "# vynil-lint-disable package/foo\nkey: value\n# vynil-lint-enable package/foo\nother: value\n";
        let map = parse_inline_disables(src);
        assert!(!map.contains_key(&1));
        assert!(map.get(&2).is_some_and(|s| s.contains("package/foo")));
        assert!(!map.contains_key(&3));
        assert!(!map.contains_key(&4));
    }

    #[test]
    fn block_and_inline_combine_on_same_line() {
        let src = "// vynil-lint-disable rhai/foo\ncode(); // vynil-lint-disable rhai/bar\n// vynil-lint-enable rhai/foo\n";
        let map = parse_inline_disables(src);
        let line2 = map.get(&2).expect("line 2 should have disables");
        assert!(line2.contains("rhai/foo"), "block disable must propagate");
        assert!(line2.contains("rhai/bar"), "inline disable must also apply");
    }

    #[test]
    fn block_disable_multiple_rules() {
        let src = "// vynil-lint-disable rhai/foo, rhai/bar\ncode();\n// vynil-lint-enable rhai/foo\ncode2();\n// vynil-lint-enable rhai/bar\n";
        let map = parse_inline_disables(src);
        let line2 = map.get(&2).expect("line 2 should have disables");
        assert!(line2.contains("rhai/foo"));
        assert!(line2.contains("rhai/bar"));
        // After enabling foo, only bar remains
        let line4 = map.get(&4).expect("line 4 should have rhai/bar");
        assert!(!line4.contains("rhai/foo"), "rhai/foo should be re-enabled");
        assert!(line4.contains("rhai/bar"), "rhai/bar still disabled");
    }
}
