use crate::linting::{LintConfig, LintFinding, LintLevel, parse_inline_disables};
use common::{
    handlebarshandler::{HbsPath, Parameter, PathSeg, Template, TemplateElement, is_native_hbs_helper},
    vynilpackage::VynilPackageSource,
};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

const KNOWN_HBS_ROOTS: &[&str] = &[
    "cluster",
    "controller",
    "instance",
    "values",
    "defaults",
    "package_dir",
    "config_dir",
    "extra",
    "this",
    "tenant",
    "system",
    "service",
    "namespace",
];

/// Shared empty rule set for lookups on line 0.
static NO_INLINE_DISABLES: std::sync::LazyLock<HashSet<String>> = std::sync::LazyLock::new(HashSet::new);

pub struct HbsChecker<'a> {
    package_dir: &'a Path,
    pkg: &'a VynilPackageSource,
    config: &'a LintConfig,
    defined_helpers: HashSet<String>,
    used_helpers: HashSet<String>,
    defined_partials: HashSet<String>,
    used_partials: HashSet<String>,
    used_values: HashSet<String>,
    used_value_paths: HashSet<String>,
    used_images: HashSet<String>,
    used_resources: HashSet<String>,
}

impl<'a> HbsChecker<'a> {
    pub fn new(package_dir: &'a Path, pkg: &'a VynilPackageSource, config: &'a LintConfig) -> Self {
        let mut checker = HbsChecker {
            package_dir,
            pkg,
            config,
            defined_helpers: HashSet::new(),
            used_helpers: HashSet::new(),
            defined_partials: HashSet::new(),
            used_partials: HashSet::new(),
            used_values: HashSet::new(),
            used_value_paths: HashSet::new(),
            used_images: HashSet::new(),
            used_resources: HashSet::new(),
        };

        // Scan handlebars/helpers/ for defined helpers
        let helpers_dir = package_dir.join("handlebars/helpers");
        if helpers_dir.is_dir()
            && let Ok(entries) = std::fs::read_dir(&helpers_dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "rhai")
                    && let Some(name) = path.file_stem().and_then(|n| n.to_str())
                {
                    checker.defined_helpers.insert(name.to_string());
                }
            }
        }

        // Scan handlebars/partials/ for defined partials
        let partials_dir = package_dir.join("handlebars/partials");
        if partials_dir.is_dir()
            && let Ok(entries) = std::fs::read_dir(&partials_dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "hbs")
                    && let Some(name) = path.file_stem().and_then(|n| n.to_str())
                {
                    checker.defined_partials.insert(name.to_string());
                }
            }
        }

        checker
    }

    pub fn check_file(&mut self, file: &Path, source: &str) -> Vec<LintFinding> {
        let mut findings = Vec::new();
        let inline_disables = parse_inline_disables(source);

        // Check 1: hbs/syntax
        match Template::compile(source) {
            Ok(template) => {
                let (
                    walker_findings,
                    used_helpers,
                    used_partials,
                    used_values,
                    used_value_paths,
                    used_images,
                    used_resources,
                ) = {
                    let mut walker = HelperWalker::new(
                        file,
                        self.config.clone(),
                        inline_disables,
                        self.pkg,
                        self.defined_helpers.clone(),
                        self.defined_partials.clone(),
                    );
                    walker.walk(&template);
                    (
                        walker.findings,
                        walker.used_helpers,
                        walker.used_partials,
                        walker.used_values,
                        walker.used_value_paths,
                        walker.used_images,
                        walker.used_resources,
                    )
                };

                findings.extend(walker_findings);
                self.used_helpers.extend(used_helpers);
                self.used_partials.extend(used_partials);
                self.used_values.extend(used_values);
                self.used_value_paths.extend(used_value_paths);
                self.used_images.extend(used_images);
                self.used_resources.extend(used_resources);
            }
            Err(e) => {
                if let Some(level) =
                    self.config
                        .resolve_level("hbs/syntax", file, LintLevel::Error, &HashSet::new())
                {
                    findings.push(LintFinding {
                        rule: "hbs/syntax".to_string(),
                        level,
                        file: file.to_path_buf(),
                        line: None,
                        message: format!("Syntax error: {e}"),
                    });
                }
            }
        }

        findings
    }

    pub fn finalize(&self) -> Vec<LintFinding> {
        let mut findings = Vec::new();

        // Check unused helpers
        for helper in &self.defined_helpers {
            if !self.used_helpers.contains(helper)
                && let Some(level) = self.config.resolve_level(
                    "hbs/unused-helper",
                    &PathBuf::from(&format!("handlebars/helpers/{helper}.rhai")),
                    LintLevel::Warn,
                    &HashSet::new(),
                )
            {
                findings.push(LintFinding {
                    rule: "hbs/unused-helper".to_string(),
                    level,
                    file: PathBuf::from(&format!("handlebars/helpers/{helper}.rhai")),
                    line: None,
                    message: format!("Helper `{helper}` defined but never used"),
                });
            }
        }

        // Check unused partials
        for partial in &self.defined_partials {
            if !self.used_partials.contains(partial)
                && let Some(level) = self.config.resolve_level(
                    "hbs/unused-partial",
                    &PathBuf::from(&format!("handlebars/partials/{partial}.hbs")),
                    LintLevel::Warn,
                    &HashSet::new(),
                )
            {
                findings.push(LintFinding {
                    rule: "hbs/unused-partial".to_string(),
                    level,
                    file: PathBuf::from(&format!("handlebars/partials/{partial}.hbs")),
                    line: None,
                    message: format!("Partial `{partial}` defined but never used"),
                });
            }
        }

        self.check_unused_options(&mut findings);
        self.check_unused_images(&mut findings);
        self.check_unused_resources(&mut findings);

        findings
    }

    fn check_unused_options(&self, findings: &mut Vec<LintFinding>) {
        // Check unused options
        if let Some(options) = &self.pkg.options {
            let yaml_path = self.package_dir.join("package.yaml");
            let line_numbers = super::find_option_line_numbers(&yaml_path);
            let yaml_inline_disables = std::fs::read_to_string(&yaml_path)
                .map(|src| parse_inline_disables(&src))
                .unwrap_or_default();
            for (key, schema) in options {
                let line = line_numbers.get(key).copied();
                let disables = line
                    .and_then(|l| yaml_inline_disables.get(&l))
                    .cloned()
                    .unwrap_or_default();
                if !self.used_values.contains(key)
                    && let Some(level) = self.config.resolve_level(
                        "hbs/unused-option",
                        &PathBuf::from("package.yaml"),
                        LintLevel::Warn,
                        &disables,
                    )
                {
                    findings.push(LintFinding {
                        rule: "hbs/unused-option".to_string(),
                        level,
                        file: PathBuf::from("package.yaml"),
                        line,
                        message: format!("Option `{key}` defined but never used"),
                    });
                }

                // Check unused fields of object-type options
                let mut leaf_paths = Vec::new();
                collect_option_leaf_paths(key, schema, &mut leaf_paths);
                for leaf_path in leaf_paths {
                    if !is_value_path_covered(&leaf_path, &self.used_value_paths)
                        && let Some(level) = self.config.resolve_level(
                            "hbs/unused-option-field",
                            &PathBuf::from("package.yaml"),
                            LintLevel::Warn,
                            &disables,
                        )
                    {
                        findings.push(LintFinding {
                            rule: "hbs/unused-option-field".to_string(),
                            level,
                            file: PathBuf::from("package.yaml"),
                            line,
                            message: format!("Option `{leaf_path}` defined but never used"),
                        });
                    }
                }
            }
        }
    }

    fn check_unused_images(&self, findings: &mut Vec<LintFinding>) {
        // Check unused images
        if let Some(images) = &self.pkg.images {
            let yaml_path = self.package_dir.join("package.yaml");
            let line_numbers = super::find_section_key_line_numbers(&yaml_path, "images");
            let yaml_inline_disables = std::fs::read_to_string(&yaml_path)
                .map(|src| parse_inline_disables(&src))
                .unwrap_or_default();
            for key in images.keys() {
                let line = line_numbers.get(key).copied();
                let disables = line
                    .and_then(|l| yaml_inline_disables.get(&l))
                    .cloned()
                    .unwrap_or_default();
                if !self.used_images.contains(key)
                    && let Some(level) = self.config.resolve_level(
                        "hbs/unused-image",
                        &PathBuf::from("package.yaml"),
                        LintLevel::Warn,
                        &disables,
                    )
                {
                    findings.push(LintFinding {
                        rule: "hbs/unused-image".to_string(),
                        level,
                        file: PathBuf::from("package.yaml"),
                        line,
                        message: format!("Image `{key}` defined but never used"),
                    });
                }
            }
        }
    }

    fn check_unused_resources(&self, findings: &mut Vec<LintFinding>) {
        // Check unused resources
        if let Some(resources) = &self.pkg.resources {
            let yaml_path = self.package_dir.join("package.yaml");
            let line_numbers = super::find_section_key_line_numbers(&yaml_path, "resources");
            let yaml_inline_disables = std::fs::read_to_string(&yaml_path)
                .map(|src| parse_inline_disables(&src))
                .unwrap_or_default();
            for key in resources.keys() {
                let line = line_numbers.get(key).copied();
                let disables = line
                    .and_then(|l| yaml_inline_disables.get(&l))
                    .cloned()
                    .unwrap_or_default();
                if !self.used_resources.contains(key)
                    && let Some(level) = self.config.resolve_level(
                        "hbs/unused-resource",
                        &PathBuf::from("package.yaml"),
                        LintLevel::Warn,
                        &disables,
                    )
                {
                    findings.push(LintFinding {
                        rule: "hbs/unused-resource".to_string(),
                        level,
                        file: PathBuf::from("package.yaml"),
                        line,
                        message: format!("Resource `{key}` defined but never used"),
                    });
                }
            }
        }
    }

    pub fn scan_rhai_for_values(&mut self, source: &str) {
        let aliases = collect_rhai_value_aliases(source);
        for part in source.split("context.values.").skip(1) {
            let path: String = part
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                .collect();
            let path = path.trim_end_matches('.').to_string();
            if !path.is_empty() {
                if let Some(key) = path.split('.').next() {
                    self.used_values.insert(key.to_string());
                }
                self.used_value_paths.insert(path);
            }
        }
        for (name, paths) in &aliases {
            let needle = format!("{name}.");
            for (idx, _) in source.match_indices(&needle) {
                if source
                    .get(..idx)
                    .is_some_and(|before| before.chars().next_back().is_some_and(is_rhai_path_char))
                {
                    continue;
                }
                let Some(tail) = source.get(idx..).and_then(|from| from.get(needle.len()..)) else {
                    continue;
                };
                for base in paths {
                    self.record_aliased_value_path(tail, base);
                }
            }
        }
    }

    /// Record `base.<tail>` for an access that starts at the head of `tail`, using the
    /// same key-cut rule as the direct `context.values.` scanner.
    fn record_aliased_value_path(&mut self, tail: &str, base: &str) {
        let tail: String = tail
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
            .collect();
        let tail = tail.trim_end_matches('.');
        if tail.is_empty() {
            return;
        }
        let path = join_value_path(base, tail);
        if let Some(key) = path.split('.').next() {
            self.used_values.insert(key.to_string());
        }
        self.used_value_paths.insert(path);
    }

    pub fn scan_rhai_for_images(&mut self, source: &str) {
        for part in source.split("context.images.").skip(1) {
            let key: String = part
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !key.is_empty() {
                self.used_images.insert(key);
            }
        }
    }

    pub fn scan_rhai_for_resources(&mut self, source: &str) {
        for prefix in &["context.instance.resources."] {
            for part in source.split(prefix).skip(1) {
                let key: String = part
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !key.is_empty() {
                    self.used_resources.insert(key);
                }
            }
        }
    }

    pub fn scan_rhai_for_partials(&mut self, source: &str) {
        for part in source.split("{{>").skip(1) {
            let name: String = part
                .trim_start_matches(|c: char| c.is_whitespace())
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '}')
                .collect();
            if !name.is_empty() {
                self.used_partials.insert(name);
            }
        }
    }
}

fn collect_option_leaf_paths(prefix: &str, schema: &serde_json::Value, paths: &mut Vec<String>) {
    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        for (key, sub_schema) in props {
            let child_path = format!("{prefix}.{key}");
            if sub_schema.get("properties").is_some() {
                collect_option_leaf_paths(&child_path, sub_schema, paths);
            } else {
                paths.push(child_path);
            }
        }
    }
}

fn is_value_path_covered(path: &str, used_paths: &HashSet<String>) -> bool {
    if used_paths.contains(path) {
        return true;
    }
    let mut current = path.to_string();
    while let Some(idx) = current.rfind('.') {
        current.truncate(idx);
        if used_paths.contains(&current) {
            return true;
        }
    }
    false
}

/// Join a values base path (empty for the `context.values` root) with a dotted suffix.
fn join_value_path(base: &str, suffix: &str) -> String {
    if base.is_empty() {
        suffix.to_string()
    } else if suffix.is_empty() {
        base.to_string()
    } else {
        format!("{base}.{suffix}")
    }
}

/// Base of an alias declaration: either the literal `context.values` root, or another
/// declared name (an alias of an alias).
enum AliasBase {
    Root,
    Name(String),
}

/// One parsed `let NAME = <base>.<suffix>;` declaration whose right-hand side names
/// `context.values` or a candidate alias name.
struct AliasRule {
    name: String,
    base: AliasBase,
    suffix: String,
}

/// Collect the simple aliases of `context.values` declared in a Rhai source:
/// `let X = context.values;` and `let X = context.values.<path>;`, where `<path>` is a
/// plain dotted key path ending at the `;` (indexing or a call disqualifies the
/// declaration). An alias of an alias (`let W = X.<path>;`) is followed. Scope is the
/// whole file with no lexical analysis: a redefinition of `X` ADDS to its earlier paths,
/// and an access declared before its alias is still seen. Declarations whose base never
/// resolves (unknown name, alias cycles) contribute nothing.
fn collect_rhai_value_aliases(source: &str) -> HashMap<String, HashSet<String>> {
    let mut pending: Vec<AliasRule> = Vec::new();
    for (idx, _) in source.match_indices("let") {
        if source
            .get(..idx)
            .is_some_and(|before| before.chars().next_back().is_some_and(is_rhai_path_char))
        {
            continue;
        }
        let Some(after) = source.get(idx..).and_then(|from| from.get("let".len()..)) else {
            continue;
        };
        if !after.chars().next().is_some_and(|c| c.is_ascii_whitespace()) {
            continue;
        }
        if let Some(rule) = parse_rhai_let_alias(after) {
            pending.push(rule);
        }
    }

    let mut resolved: HashMap<String, HashSet<String>> = HashMap::new();
    let mut progressed = true;
    while progressed {
        progressed = false;
        let rules = std::mem::take(&mut pending);
        for rule in rules {
            let bases = match &rule.base {
                AliasBase::Root => Some(vec![String::new()]),
                AliasBase::Name(n) => resolved.get(n).map(|s| s.iter().cloned().collect::<Vec<_>>()),
            };
            if let Some(bases) = bases {
                progressed = true;
                let entry = resolved.entry(rule.name).or_default();
                for base in bases {
                    entry.insert(join_value_path(&base, &rule.suffix));
                }
            } else {
                pending.push(rule);
            }
        }
    }
    resolved
}

/// Parse `NAME = <rhs>;` from the text right after the `let` keyword. Returns `None`
/// unless the whole declaration has the simple alias shape (plain identifier, `=`,
/// `context.values` or a dotted `<name>.<path>` right-hand side, terminated by `;`).
fn parse_rhai_let_alias(rest: &str) -> Option<AliasRule> {
    let mut cur = Cursor::new(rest);
    cur.skip_ws();
    let name = cur.take_while(is_rhai_ident_char);
    if name.is_empty() {
        return None;
    }
    cur.skip_ws();
    if cur.first() != Some('=') {
        return None;
    }
    cur.eat('=');
    cur.skip_ws();

    let (base, suffix) = if cur.rest_starts_with("context.values") {
        cur.eat_str("context.values");
        if cur.first().is_some_and(is_rhai_ident_char) {
            return None;
        }
        (AliasBase::Root, take_value_path(&mut cur)?)
    } else {
        let base_name = cur.take_while(is_rhai_ident_char);
        if base_name.is_empty() || cur.first() != Some('.') {
            return None;
        }
        (AliasBase::Name(base_name), take_value_path(&mut cur)?)
    };
    cur.skip_ws();
    if cur.first() != Some(';') {
        return None;
    }
    Some(AliasRule { name, base, suffix })
}

/// Consume a leading `.` plus dotted key path, or nothing at all. Returns `None` when a
/// dot is present but no key character follows it. Trailing dots are trimmed like the
/// direct scanner does.
fn take_value_path(cur: &mut Cursor) -> Option<String> {
    if !cur.eat('.') {
        return Some(String::new());
    }
    let raw = cur.take_while(is_rhai_path_char);
    let trimmed = raw.trim_end_matches('.');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn is_rhai_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Characters that glue a Rhai identifier to its neighbours: matching one just before an
/// occurrence means the occurrence is part of a longer token, not a standalone access.
fn is_rhai_path_char(c: char) -> bool {
    is_rhai_ident_char(c) || c == '.'
}

/// Minimal text cursor over `&str` built only on boundary-safe operations
/// (`get`/`strip_prefix`), so no arithmetic or raw slicing can panic.
struct Cursor<'a>(&'a str);

impl<'a> Cursor<'a> {
    const fn new(text: &'a str) -> Self {
        Self(text)
    }

    fn first(&self) -> Option<char> {
        self.0.chars().next()
    }

    fn rest_starts_with(&self, lit: &str) -> bool {
        self.0.starts_with(lit)
    }

    fn skip_ws(&mut self) {
        while let Some(rest) = self.0.strip_prefix(|c: char| c.is_ascii_whitespace()) {
            self.0 = rest;
        }
    }

    fn eat(&mut self, target: char) -> bool {
        if let Some(rest) = self.0.strip_prefix(target) {
            self.0 = rest;
            return true;
        }
        false
    }

    fn eat_str(&mut self, lit: &str) -> bool {
        if let Some(rest) = self.0.strip_prefix(lit) {
            self.0 = rest;
            return true;
        }
        false
    }

    fn take_while(&mut self, pred: impl Fn(char) -> bool) -> String {
        let mut out = String::new();
        while let Some(c) = self.first() {
            if !pred(c) {
                break;
            }
            out.push(c);
            self.0 = self.0.get(c.len_utf8()..).unwrap_or_default();
        }
        out
    }
}

struct HelperWalker<'a> {
    file: PathBuf,
    config: LintConfig,
    inline_disables: HashMap<usize, HashSet<String>>,
    findings: Vec<LintFinding>,
    defined_helpers: HashSet<String>,
    defined_partials: HashSet<String>,
    pub used_helpers: HashSet<String>,
    pub used_partials: HashSet<String>,
    pub used_values: HashSet<String>,
    pub used_value_paths: HashSet<String>,
    pub used_images: HashSet<String>,
    pub used_resources: HashSet<String>,
    pkg: &'a VynilPackageSource,
}

impl<'a> HelperWalker<'a> {
    fn new(
        file: &Path,
        config: LintConfig,
        inline_disables: HashMap<usize, HashSet<String>>,
        pkg: &'a VynilPackageSource,
        defined_helpers: HashSet<String>,
        defined_partials: HashSet<String>,
    ) -> Self {
        HelperWalker {
            file: file.to_path_buf(),
            config,
            inline_disables,
            findings: Vec::new(),
            defined_helpers,
            defined_partials,
            used_helpers: HashSet::new(),
            used_partials: HashSet::new(),
            used_values: HashSet::new(),
            used_value_paths: HashSet::new(),
            used_images: HashSet::new(),
            used_resources: HashSet::new(),
            pkg,
        }
    }

    fn walk(&mut self, template: &Template) {
        Self::walk_template(template, &mut |elem, line| {
            self.visit_element(elem, line);
        });
    }

    fn walk_template<F>(tpl: &Template, visitor: &mut F)
    where
        F: FnMut(&TemplateElement, Option<usize>),
    {
        for (i, elem) in tpl.elements.iter().enumerate() {
            let line = tpl.mapping.get(i).map(|m| m.0);
            visitor(elem, line);
            match elem {
                TemplateElement::HelperBlock(h) => {
                    if let Some(t) = &h.template {
                        Self::walk_template(t, visitor);
                    }
                    if let Some(t) = &h.inverse {
                        Self::walk_template(t, visitor);
                    }
                }
                TemplateElement::DecoratorBlock(d) | TemplateElement::PartialBlock(d) => {
                    if let Some(t) = &d.template {
                        Self::walk_template(t, visitor);
                    }
                }
                _ => {}
            }
        }
    }

    fn visit_element(&mut self, elem: &TemplateElement, line: Option<usize>) {
        match elem {
            TemplateElement::HelperBlock(h) => {
                self.check_helper(&h.name, &h.params, line);
                self.check_image_resource_helper(&h.name, &h.params, line);
                self.check_path(&h.name, line);
                for param in &h.params {
                    self.check_values_path(param, line);
                    self.check_root_var(param, line);
                }
            }
            TemplateElement::Expression(e) | TemplateElement::HtmlExpression(e) => {
                self.check_helper(&e.name, &e.params, line);
                self.check_values_path(&e.name, line);
                for param in &e.params {
                    self.check_values_path(param, line);
                    self.check_root_var(param, line);
                }
                self.check_image_resource_helper(&e.name, &e.params, line);
                self.check_path(&e.name, line);
                self.check_root_var(&e.name, line);
            }
            TemplateElement::DecoratorBlock(d) => {
                self.check_helper(&d.name, &d.params, line);
            }
            TemplateElement::PartialExpression(d) | TemplateElement::PartialBlock(d) => {
                self.check_partial(&d.name, line);
            }
            _ => {}
        }
    }

    fn check_helper(&mut self, name: &Parameter, params: &[Parameter], line: Option<usize>) {
        if let Parameter::Name(helper_name) = name
            && !is_native_hbs_helper(helper_name)
        {
            self.used_helpers.insert(helper_name.clone());

            if !self.defined_helpers.contains(helper_name)
                && let Some(level) = self.config.resolve_level(
                    "hbs/unknown-helper",
                    &self.file,
                    LintLevel::Error,
                    self.inline_disables.get(&0).unwrap_or(&NO_INLINE_DISABLES),
                )
            {
                self.findings.push(LintFinding {
                    rule: "hbs/unknown-helper".to_string(),
                    level,
                    file: self.file.clone(),
                    line,
                    message: format!("Helper `{helper_name}` not found"),
                });
            }
        }

        // Check for nested subexpressions
        for param in params {
            if let Parameter::Subexpression(sub) = param {
                self.visit_element(sub.as_element(), line);
            }
        }
    }

    fn check_values_path(&mut self, param: &Parameter, line: Option<usize>) {
        if let Parameter::Path(path) = param
            && let HbsPath::Relative((segs, _)) = path
            && !segs.is_empty()
            && let Some(PathSeg::Named(first)) = segs.first()
            && first == "values"
            && segs.len() >= 2
            && let Some(PathSeg::Named(key)) = segs.get(1)
        {
            self.used_values.insert(key.clone());

            // Track the full dot-separated path for leaf detection
            let path_parts: Vec<&str> = segs
                .iter()
                .skip(1)
                .filter_map(|s| {
                    if let PathSeg::Named(n) = s {
                        Some(n.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            if !path_parts.is_empty() {
                self.used_value_paths.insert(path_parts.join("."));
            }

            if let Some(options) = &self.pkg.options
                && !options.contains_key(key)
                && let Some(level) = self.config.resolve_level(
                    "hbs/unknown-value",
                    &self.file,
                    LintLevel::Error,
                    self.inline_disables.get(&0).unwrap_or(&NO_INLINE_DISABLES),
                )
            {
                self.findings.push(LintFinding {
                    rule: "hbs/unknown-value".to_string(),
                    level,
                    file: self.file.clone(),
                    line,
                    message: format!("Unknown value key `{key}`"),
                });
            }
        }
    }

    fn check_image_resource_helper(&mut self, name: &Parameter, params: &[Parameter], line: Option<usize>) {
        if let Parameter::Name(helper_name) = name {
            let rule = if helper_name == "image_from_ctx" {
                Some(("hbs/unknown-image", "images"))
            } else if helper_name == "resources_from_ctx" {
                Some(("hbs/unknown-resource", "resources"))
            } else {
                None
            };

            if let Some((rule_name, field_name)) = rule
                && params.len() > 1
                && let Some(Parameter::Literal(json_val)) = params.get(1)
                && let Some(key) = json_val.as_str()
            {
                let has_key = if field_name == "images" {
                    self.pkg
                        .images
                        .as_ref()
                        .is_some_and(|imgs| imgs.contains_key(key))
                } else {
                    self.pkg
                        .resources
                        .as_ref()
                        .is_some_and(|res| res.contains_key(key))
                };

                // Track the used key regardless of whether it exists
                if field_name == "images" {
                    self.used_images.insert(key.to_string());
                } else {
                    self.used_resources.insert(key.to_string());
                }

                if !has_key
                    && ((field_name == "images" && self.pkg.images.is_some())
                        || (field_name == "resources" && self.pkg.resources.is_some()))
                    && let Some(level) = self.config.resolve_level(
                        rule_name,
                        &self.file,
                        LintLevel::Error,
                        self.inline_disables.get(&0).unwrap_or(&NO_INLINE_DISABLES),
                    )
                {
                    self.findings.push(LintFinding {
                        rule: rule_name.to_string(),
                        level,
                        file: self.file.clone(),
                        line,
                        message: format!("Unknown {} key `{}`", field_name.trim_end_matches('s'), key),
                    });
                }
            }
        }
    }

    fn check_path(&mut self, param: &Parameter, line: Option<usize>) {
        if let Parameter::Path(path) = param
            && let HbsPath::Relative((segs, _)) = path
            && !segs.is_empty()
            && let Some(PathSeg::Named(first)) = segs.first()
            && first == "tenant"
            && self.pkg.metadata.usage == common::vynilpackage::VynilPackageType::System
            && let Some(level) = self.config.resolve_level(
                "hbs/wrong-package-type",
                &self.file,
                LintLevel::Warn,
                self.inline_disables.get(&0).unwrap_or(&NO_INLINE_DISABLES),
            )
        {
            self.findings.push(LintFinding {
                rule: "hbs/wrong-package-type".to_string(),
                level,
                file: self.file.clone(),
                line,
                message: "Accessing `tenant` in a System package is not allowed".to_string(),
            });
        }
    }

    fn check_root_var(&mut self, param: &Parameter, line: Option<usize>) {
        if let Parameter::Path(path) = param
            && let HbsPath::Relative((segs, _)) = path
            && segs.len() >= 2
            && let Some(PathSeg::Named(first)) = segs.first()
            && !first.starts_with('@')
            && !KNOWN_HBS_ROOTS.contains(&first.as_str())
        {
            // Specific check: `context.<known_root>` is a common mistake — the `context.` prefix
            // used in Rhai scripts is not available in HBS templates.
            if first == "context"
                && let Some(PathSeg::Named(second)) = segs.get(1)
                && KNOWN_HBS_ROOTS.contains(&second.as_str())
                && let Some(level) = self.config.resolve_level(
                    "hbs/context-prefix",
                    &self.file,
                    LintLevel::Error,
                    self.inline_disables.get(&0).unwrap_or(&NO_INLINE_DISABLES),
                )
            {
                self.findings.push(LintFinding {
                    rule: "hbs/context-prefix".to_string(),
                    level,
                    file: self.file.clone(),
                    line,
                    message: format!(
                        "Use `{second}` instead of `context.{second}`; the `context.` prefix is not valid in templates",
                    ),
                });
                return;
            }

            if let Some(level) = self.config.resolve_level(
                "hbs/unknown-root-variable",
                &self.file,
                LintLevel::Warn,
                self.inline_disables.get(&0).unwrap_or(&NO_INLINE_DISABLES),
            ) {
                self.findings.push(LintFinding {
                    rule: "hbs/unknown-root-variable".to_string(),
                    level,
                    file: self.file.clone(),
                    line,
                    message: format!(
                        "Unknown root variable `{}` in template path — expected one of: {}",
                        first,
                        KNOWN_HBS_ROOTS.join(", ")
                    ),
                });
            }
        }
    }

    fn check_partial(&mut self, name: &Parameter, line: Option<usize>) {
        let partial_name = match name {
            Parameter::Name(s) => s.clone(),
            Parameter::Literal(v) => v.as_str().unwrap_or("").to_string(),
            _ => return,
        };

        self.used_partials.insert(partial_name.clone());

        if !self.defined_partials.contains(&partial_name)
            && let Some(level) = self.config.resolve_level(
                "hbs/unknown-partial",
                &self.file,
                LintLevel::Error,
                self.inline_disables.get(&0).unwrap_or(&NO_INLINE_DISABLES),
            )
        {
            self.findings.push(LintFinding {
                rule: "hbs/unknown-partial".to_string(),
                level,
                file: self.file.clone(),
                line,
                message: format!("Partial `{partial_name}` not found"),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::vynilpackage::VynilPackageMeta;

    struct TestChecker {
        checker: HbsChecker<'static>,
    }

    impl TestChecker {
        fn new(defined_helpers: Vec<&str>, defined_partials: Vec<&str>) -> Self {
            let pkg = Box::leak(Box::new(create_dummy_pkg()));
            let config = Box::leak(Box::new(LintConfig::default()));
            let checker = HbsChecker {
                package_dir: Path::new("."),
                pkg,
                config,
                defined_helpers: defined_helpers
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect(),
                used_helpers: HashSet::new(),
                defined_partials: defined_partials
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect(),
                used_partials: HashSet::new(),
                used_values: HashSet::new(),
                used_value_paths: HashSet::new(),
                used_images: HashSet::new(),
                used_resources: HashSet::new(),
            };
            drop(defined_helpers);
            drop(defined_partials);
            Self { checker }
        }
    }

    fn create_dummy_pkg() -> VynilPackageSource {
        VynilPackageSource {
            apiVersion: "v1".to_string(),
            kind: "VynilPackage".to_string(),
            metadata: VynilPackageMeta {
                name: "test".to_string(),
                category: "system".to_string(),
                description: "test package".to_string(),
                app_version: None,
                usage: common::vynilpackage::VynilPackageType::System,
                features: vec![],
                backup_affinity: None,
            },
            requirements: vec![],
            recommandations: None,
            options: None,
            images: None,
            resources: None,
            value_script: None,
        }
    }

    #[test]
    fn hbs_syntax_error_detected() {
        let mut test = TestChecker::new(vec![], vec![]);
        let source = "{{#if foo}}oops"; // bloc non fermé
        let findings = test.checker.check_file(Path::new("test.hbs"), source);

        assert!(!findings.is_empty());
        assert!(findings.iter().any(|f| f.rule == "hbs/syntax"));
        assert!(findings.iter().any(|f| f.level == LintLevel::Error));
    }

    #[test]
    fn known_helper_no_finding() {
        let mut test = TestChecker::new(vec![], vec![]);
        let source = "{{base64_encode val}}";
        let findings = test.checker.check_file(Path::new("test.hbs"), source);

        assert!(!findings.iter().any(|f| f.rule == "hbs/unknown-helper"));
    }

    #[test]
    fn read_to_str_helper_no_finding() {
        let mut test = TestChecker::new(vec![], vec![]);
        let source = r#"{{read_to_str "/some/file"}}"#;
        let findings = test.checker.check_file(Path::new("test.hbs"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "hbs/unknown-helper"),
            "read_to_str should be known (registered by handlebars_misc_helpers)"
        );
    }

    #[test]
    fn unknown_helper_produces_error() {
        let mut test = TestChecker::new(vec![], vec![]);
        let source = "{{unknown_helper_xyz val}}";
        let findings = test.checker.check_file(Path::new("test.hbs"), source);

        assert!(findings.iter().any(|f| f.rule == "hbs/unknown-helper"));
    }

    #[test]
    fn defined_custom_helper_no_error() {
        let mut test = TestChecker::new(vec!["my_custom_helper"], vec![]);
        let source = "{{my_custom_helper val}}";
        let findings = test.checker.check_file(Path::new("test.hbs"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "hbs/unknown-helper"),
            "Custom helper defined in handlebars/helpers/ should not be flagged"
        );
    }

    #[test]
    fn used_partial_no_warning() {
        let mut test = TestChecker::new(vec![], vec!["mypartial"]);
        let source = "{{> mypartial}}";
        test.checker.check_file(Path::new("test.hbs"), source);
        let final_findings = test.checker.finalize();

        assert!(
            !final_findings
                .iter()
                .any(|f| f.rule == "hbs/unused-partial" && f.message.contains("mypartial"))
        );
    }

    #[test]
    fn unused_partial_produces_warning() {
        let mut test = TestChecker::new(vec![], vec!["unused_partial"]);
        let source = "{{foo}}";
        test.checker.check_file(Path::new("test.hbs"), source);
        let final_findings = test.checker.finalize();

        assert!(
            final_findings
                .iter()
                .any(|f| f.rule == "hbs/unused-partial" && f.message.contains("unused_partial"))
        );
    }

    #[test]
    fn unknown_partial_produces_error() {
        let mut test = TestChecker::new(vec![], vec![]);
        let source = "{{> nonexistent_partial}}";
        let findings = test.checker.check_file(Path::new("test.hbs"), source);

        assert!(findings.iter().any(|f| f.rule == "hbs/unknown-partial"));
    }

    #[test]
    fn known_value_key_no_finding() {
        let pkg = Box::leak(Box::new(create_pkg_with_options()));
        let config = Box::leak(Box::new(LintConfig::default()));
        let mut checker = HbsChecker {
            package_dir: Path::new("."),
            pkg,
            config,
            defined_helpers: HashSet::new(),
            used_helpers: HashSet::new(),
            defined_partials: HashSet::new(),
            used_partials: HashSet::new(),
            used_values: HashSet::new(),
            used_value_paths: HashSet::new(),
            used_images: HashSet::new(),
            used_resources: HashSet::new(),
        };

        let source = "{{values.port}}";
        let findings = checker.check_file(Path::new("test.hbs"), source);

        assert!(!findings.iter().any(|f| f.rule == "hbs/unknown-value"));
    }

    #[test]
    fn unknown_value_key_produces_error() {
        let pkg = Box::leak(Box::new(create_pkg_with_options()));
        let config = Box::leak(Box::new(LintConfig::default()));
        let mut checker = HbsChecker {
            package_dir: Path::new("."),
            pkg,
            config,
            defined_helpers: HashSet::new(),
            used_helpers: HashSet::new(),
            defined_partials: HashSet::new(),
            used_partials: HashSet::new(),
            used_values: HashSet::new(),
            used_value_paths: HashSet::new(),
            used_images: HashSet::new(),
            used_resources: HashSet::new(),
        };

        let source = "{{values.unknown_key}}";
        let findings = checker.check_file(Path::new("test.hbs"), source);

        assert!(findings.iter().any(|f| f.rule == "hbs/unknown-value"));
    }

    #[test]
    fn values_check_skipped_when_no_options() {
        let pkg = Box::leak(Box::new(create_dummy_pkg()));
        let config = Box::leak(Box::new(LintConfig::default()));
        let mut checker = HbsChecker {
            package_dir: Path::new("."),
            pkg,
            config,
            defined_helpers: HashSet::new(),
            used_helpers: HashSet::new(),
            defined_partials: HashSet::new(),
            used_partials: HashSet::new(),
            used_values: HashSet::new(),
            used_value_paths: HashSet::new(),
            used_images: HashSet::new(),
            used_resources: HashSet::new(),
        };

        let source = "{{values.anything}}";
        let findings = checker.check_file(Path::new("test.hbs"), source);

        assert!(!findings.iter().any(|f| f.rule == "hbs/unknown-value"));
    }

    #[test]
    fn unknown_image_key_produces_error() {
        let pkg = Box::leak(Box::new(create_pkg_with_images()));
        let config = Box::leak(Box::new(LintConfig::default()));
        let mut checker = HbsChecker {
            package_dir: Path::new("."),
            pkg,
            config,
            defined_helpers: HashSet::new(),
            used_helpers: HashSet::new(),
            defined_partials: HashSet::new(),
            used_partials: HashSet::new(),
            used_values: HashSet::new(),
            used_value_paths: HashSet::new(),
            used_images: HashSet::new(),
            used_resources: HashSet::new(),
        };

        let source = "{{image_from_ctx this \"missing\"}}";
        let findings = checker.check_file(Path::new("test.hbs"), source);

        assert!(findings.iter().any(|f| f.rule == "hbs/unknown-image"));
    }

    #[test]
    fn unknown_resource_key_produces_error() {
        let pkg = Box::leak(Box::new(create_pkg_with_resources()));
        let config = Box::leak(Box::new(LintConfig::default()));
        let mut checker = HbsChecker {
            package_dir: Path::new("."),
            pkg,
            config,
            defined_helpers: HashSet::new(),
            used_helpers: HashSet::new(),
            defined_partials: HashSet::new(),
            used_partials: HashSet::new(),
            used_values: HashSet::new(),
            used_value_paths: HashSet::new(),
            used_images: HashSet::new(),
            used_resources: HashSet::new(),
        };

        let source = "{{resources_from_ctx this \"missing\"}}";
        let findings = checker.check_file(Path::new("test.hbs"), source);

        assert!(findings.iter().any(|f| f.rule == "hbs/unknown-resource"));
    }

    #[test]
    fn unknown_root_variable_produces_warning() {
        let mut test = TestChecker::new(vec![], vec![]);
        let source = "{{ins_bug_tance.appslug}}-controller";
        let findings = test.checker.check_file(Path::new("test.hbs"), source);

        assert!(
            findings.iter().any(|f| f.rule == "hbs/unknown-root-variable"),
            "Expected hbs/unknown-root-variable for 'ins_bug_tance'"
        );
    }

    #[test]
    fn context_tenant_prefix_produces_error() {
        let mut test = TestChecker::new(vec![], vec![]);
        let source = "{{context.tenant.name}}";
        let findings = test
            .checker
            .check_file(Path::new("posts/RestEndPoint_test.yaml.hbs"), source);

        assert!(
            findings
                .iter()
                .any(|f| f.rule == "hbs/context-prefix" && f.level == LintLevel::Error),
            "Expected hbs/context-prefix error for 'context.tenant'"
        );
        assert!(
            !findings.iter().any(|f| f.rule == "hbs/unknown-root-variable"),
            "hbs/unknown-root-variable should not also fire when hbs/context-prefix does"
        );
    }

    #[test]
    fn context_instance_prefix_produces_error() {
        let mut test = TestChecker::new(vec![], vec![]);
        let source = "{{context.instance.appslug}}";
        let findings = test.checker.check_file(Path::new("test.hbs"), source);

        assert!(
            findings.iter().any(|f| f.rule == "hbs/context-prefix"),
            "Expected hbs/context-prefix for 'context.instance'"
        );
        assert!(
            findings
                .iter()
                .any(|f| f.rule == "hbs/context-prefix" && f.message.contains("instance")),
            "Error message should mention 'instance' as the correct root"
        );
    }

    #[test]
    fn context_unknown_field_produces_generic_warning() {
        let mut test = TestChecker::new(vec![], vec![]);
        let source = "{{context.something_unknown.foo}}";
        let findings = test.checker.check_file(Path::new("test.hbs"), source);

        assert!(
            findings.iter().any(|f| f.rule == "hbs/unknown-root-variable"),
            "context.<unknown> should still produce hbs/unknown-root-variable"
        );
        assert!(
            !findings.iter().any(|f| f.rule == "hbs/context-prefix"),
            "hbs/context-prefix should not fire when second segment is not a known root"
        );
    }

    #[test]
    fn known_root_variable_no_warning() {
        let mut test = TestChecker::new(vec![], vec![]);
        let source = "{{instance.appslug}}-controller";
        let findings = test.checker.check_file(Path::new("test.hbs"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "hbs/unknown-root-variable"),
            "Known root 'instance' should not be flagged"
        );
    }

    #[test]
    fn this_root_not_flagged() {
        let mut test = TestChecker::new(vec![], vec![]);
        let source = "{{this.appslug}}";
        let findings = test.checker.check_file(Path::new("test.hbs"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "hbs/unknown-root-variable"),
            "'this' should not be flagged as unknown root"
        );
    }

    #[test]
    fn single_segment_path_not_flagged_as_unknown_root() {
        let mut test = TestChecker::new(vec![], vec![]);
        let source = "{{appslug}}";
        let findings = test.checker.check_file(Path::new("test.hbs"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "hbs/unknown-root-variable"),
            "Single-segment paths should not be flagged as unknown root"
        );
    }

    #[test]
    fn tenant_access_in_system_package_warns() {
        let pkg = Box::leak(Box::new(create_system_pkg()));
        let config = Box::leak(Box::new(LintConfig::default()));
        let mut checker = HbsChecker {
            package_dir: Path::new("."),
            pkg,
            config,
            defined_helpers: HashSet::new(),
            used_helpers: HashSet::new(),
            defined_partials: HashSet::new(),
            used_partials: HashSet::new(),
            used_values: HashSet::new(),
            used_value_paths: HashSet::new(),
            used_images: HashSet::new(),
            used_resources: HashSet::new(),
        };

        let source = "{{tenant.name}}";
        let findings = checker.check_file(Path::new("test.hbs"), source);

        assert!(findings.iter().any(|f| f.rule == "hbs/wrong-package-type"));
        assert!(findings.iter().any(|f| f.level == LintLevel::Warn));
    }

    #[test]
    fn unused_option_produces_warning() {
        let pkg = Box::leak(Box::new(create_pkg_with_options()));
        let config = Box::leak(Box::new(LintConfig::default()));
        let mut checker = HbsChecker {
            package_dir: Path::new("."),
            pkg,
            config,
            defined_helpers: HashSet::new(),
            used_helpers: HashSet::new(),
            defined_partials: HashSet::new(),
            used_partials: HashSet::new(),
            used_values: HashSet::new(),
            used_value_paths: HashSet::new(),
            used_images: HashSet::new(),
            used_resources: HashSet::new(),
        };

        // Only use "port", but package has "port" and "host"
        let source = "{{values.port}}";
        checker.check_file(Path::new("test.hbs"), source);
        let final_findings = checker.finalize();

        assert!(
            final_findings
                .iter()
                .any(|f| f.rule == "hbs/unused-option" && f.message.contains("host"))
        );
    }

    #[test]
    fn rhai_context_values_usage_suppresses_unused_option_warning() {
        let pkg = Box::leak(Box::new(create_pkg_with_options()));
        let config = Box::leak(Box::new(LintConfig::default()));
        let mut checker = HbsChecker {
            package_dir: Path::new("."),
            pkg,
            config,
            defined_helpers: HashSet::new(),
            used_helpers: HashSet::new(),
            defined_partials: HashSet::new(),
            used_partials: HashSet::new(),
            used_values: HashSet::new(),
            used_value_paths: HashSet::new(),
            used_images: HashSet::new(),
            used_resources: HashSet::new(),
        };

        // "host" is used only in a rhai file via context.values.host
        checker.scan_rhai_for_values("size: context.values.host,");
        let final_findings = checker.finalize();

        assert!(
            !final_findings
                .iter()
                .any(|f| f.rule == "hbs/unused-option" && f.message.contains("host")),
            "host used in rhai should not produce unused-option warning"
        );
        // "port" is still unused
        assert!(
            final_findings
                .iter()
                .any(|f| f.rule == "hbs/unused-option" && f.message.contains("port"))
        );
    }

    fn create_pkg_with_options() -> VynilPackageSource {
        let mut options = std::collections::BTreeMap::new();
        options.insert("port".to_string(), serde_json::json!(8080));
        options.insert("host".to_string(), serde_json::json!("localhost"));

        VynilPackageSource {
            apiVersion: "v1".to_string(),
            kind: "VynilPackage".to_string(),
            metadata: VynilPackageMeta {
                name: "test".to_string(),
                category: "system".to_string(),
                description: "test package".to_string(),
                app_version: None,
                usage: common::vynilpackage::VynilPackageType::Tenant,
                features: vec![],
                backup_affinity: None,
            },
            requirements: vec![],
            recommandations: None,
            options: Some(options),
            images: None,
            resources: None,
            value_script: None,
        }
    }

    fn create_pkg_with_images() -> VynilPackageSource {
        let mut images = std::collections::BTreeMap::new();
        images.insert("app".to_string(), common::vynilpackage::Image {
            registry: "docker.io".to_string(),
            repository: "myapp".to_string(),
            tag: Some("1.0".to_string()),
        });

        VynilPackageSource {
            apiVersion: "v1".to_string(),
            kind: "VynilPackage".to_string(),
            metadata: VynilPackageMeta {
                name: "test".to_string(),
                category: "system".to_string(),
                description: "test package".to_string(),
                app_version: None,
                usage: common::vynilpackage::VynilPackageType::Tenant,
                features: vec![],
                backup_affinity: None,
            },
            requirements: vec![],
            recommandations: None,
            options: None,
            images: Some(images),
            resources: None,
            value_script: None,
        }
    }

    fn create_pkg_with_resources() -> VynilPackageSource {
        let mut resources = std::collections::BTreeMap::new();
        resources.insert("app".to_string(), common::vynilpackage::Resource {
            requests: Some(common::vynilpackage::ResourceItem {
                cpu: Some("100m".to_string()),
                memory: Some("128Mi".to_string()),
                storage: None,
            }),
            limits: Some(common::vynilpackage::ResourceItem {
                cpu: Some("500m".to_string()),
                memory: Some("512Mi".to_string()),
                storage: None,
            }),
            scaler: None,
        });

        VynilPackageSource {
            apiVersion: "v1".to_string(),
            kind: "VynilPackage".to_string(),
            metadata: VynilPackageMeta {
                name: "test".to_string(),
                category: "system".to_string(),
                description: "test package".to_string(),
                app_version: None,
                usage: common::vynilpackage::VynilPackageType::Tenant,
                features: vec![],
                backup_affinity: None,
            },
            requirements: vec![],
            recommandations: None,
            options: None,
            images: None,
            resources: Some(resources),
            value_script: None,
        }
    }

    fn create_system_pkg() -> VynilPackageSource {
        VynilPackageSource {
            apiVersion: "v1".to_string(),
            kind: "VynilPackage".to_string(),
            metadata: VynilPackageMeta {
                name: "test".to_string(),
                category: "system".to_string(),
                description: "test package".to_string(),
                app_version: None,
                usage: common::vynilpackage::VynilPackageType::System,
                features: vec![],
                backup_affinity: None,
            },
            requirements: vec![],
            recommandations: None,
            options: None,
            images: None,
            resources: None,
            value_script: None,
        }
    }

    fn create_pkg_with_object_option() -> VynilPackageSource {
        let mut options = std::collections::BTreeMap::new();
        options.insert(
            "database".to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "host": {"type": "string"},
                    "port": {"type": "integer"}
                }
            }),
        );
        VynilPackageSource {
            apiVersion: "v1".to_string(),
            kind: "VynilPackage".to_string(),
            metadata: VynilPackageMeta {
                name: "test".to_string(),
                category: "system".to_string(),
                description: "test package".to_string(),
                app_version: None,
                usage: common::vynilpackage::VynilPackageType::Tenant,
                features: vec![],
                backup_affinity: None,
            },
            requirements: vec![],
            recommandations: None,
            options: Some(options),
            images: None,
            resources: None,
            value_script: None,
        }
    }

    fn make_checker(pkg: &'static VynilPackageSource) -> HbsChecker<'static> {
        let config = Box::leak(Box::new(LintConfig::default()));
        HbsChecker {
            package_dir: Path::new("."),
            pkg,
            config,
            defined_helpers: HashSet::new(),
            used_helpers: HashSet::new(),
            defined_partials: HashSet::new(),
            used_partials: HashSet::new(),
            used_values: HashSet::new(),
            used_value_paths: HashSet::new(),
            used_images: HashSet::new(),
            used_resources: HashSet::new(),
        }
    }

    #[test]
    fn unused_option_field_produces_warning() {
        let pkg = Box::leak(Box::new(create_pkg_with_object_option()));
        let mut checker = make_checker(pkg);

        // Use only database.host; database.port is never accessed
        checker.check_file(Path::new("test.hbs"), "{{values.database.host}}");
        let findings = checker.finalize();

        assert!(
            findings
                .iter()
                .any(|f| f.rule == "hbs/unused-option-field" && f.message.contains("database.port")),
            "database.port should produce hbs/unused-option-field"
        );
        assert!(
            !findings
                .iter()
                .any(|f| f.rule == "hbs/unused-option-field" && f.message.contains("database.host")),
            "database.host should not be flagged"
        );
    }

    #[test]
    fn whole_object_used_suppresses_field_warnings() {
        let pkg = Box::leak(Box::new(create_pkg_with_object_option()));
        let mut checker = make_checker(pkg);

        // Use the whole object — all fields are implicitly used
        checker.check_file(Path::new("test.hbs"), "{{values.database}}");
        let findings = checker.finalize();

        assert!(
            !findings.iter().any(|f| f.rule == "hbs/unused-option-field"),
            "No field warnings when the whole object is used"
        );
    }

    #[test]
    fn rhai_access_suppresses_option_field_warning() {
        let pkg = Box::leak(Box::new(create_pkg_with_object_option()));
        let mut checker = make_checker(pkg);

        // database.port accessed in Rhai, database.host never accessed
        checker.scan_rhai_for_values("let p = context.values.database.port;");
        let findings = checker.finalize();

        assert!(
            !findings
                .iter()
                .any(|f| f.rule == "hbs/unused-option-field" && f.message.contains("database.port")),
            "database.port used in Rhai should not produce warning"
        );
        assert!(
            findings
                .iter()
                .any(|f| f.rule == "hbs/unused-option-field" && f.message.contains("database.host")),
            "database.host still unused should produce warning"
        );
    }

    #[test]
    fn unused_image_produces_warning() {
        let pkg = Box::leak(Box::new(create_pkg_with_images()));
        let mut checker = make_checker(pkg);

        // No HBS file uses the image
        checker.check_file(Path::new("test.hbs"), "hello world");
        let findings = checker.finalize();

        assert!(
            findings
                .iter()
                .any(|f| f.rule == "hbs/unused-image" && f.message.contains("app")),
            "Unused image should produce hbs/unused-image warning"
        );
    }

    #[test]
    fn used_image_no_warning() {
        let pkg = Box::leak(Box::new(create_pkg_with_images()));
        let mut checker = make_checker(pkg);

        checker.check_file(Path::new("test.hbs"), r#"{{image_from_ctx this "app"}}"#);
        let findings = checker.finalize();

        assert!(
            !findings.iter().any(|f| f.rule == "hbs/unused-image"),
            "Used image should not produce warning"
        );
    }

    #[test]
    fn rhai_context_images_suppresses_unused_image_warning() {
        let pkg = Box::leak(Box::new(create_pkg_with_images()));
        let mut checker = make_checker(pkg);

        checker.scan_rhai_for_images("let img = context.images.app;");
        let findings = checker.finalize();

        assert!(
            !findings.iter().any(|f| f.rule == "hbs/unused-image"),
            "Image used in Rhai should not produce warning"
        );
    }

    #[test]
    fn unused_resource_produces_warning() {
        let pkg = Box::leak(Box::new(create_pkg_with_resources()));
        let mut checker = make_checker(pkg);

        checker.check_file(Path::new("test.hbs"), "hello world");
        let findings = checker.finalize();

        assert!(
            findings
                .iter()
                .any(|f| f.rule == "hbs/unused-resource" && f.message.contains("app")),
            "Unused resource should produce hbs/unused-resource warning"
        );
    }

    #[test]
    fn used_resource_no_warning() {
        let pkg = Box::leak(Box::new(create_pkg_with_resources()));
        let mut checker = make_checker(pkg);

        checker.check_file(Path::new("test.hbs"), r#"{{resources_from_ctx this "app"}}"#);
        let findings = checker.finalize();

        assert!(
            !findings.iter().any(|f| f.rule == "hbs/unused-resource"),
            "Used resource should not produce warning"
        );
    }

    #[test]
    fn rhai_context_resources_suppresses_unused_resource_warning() {
        let pkg = Box::leak(Box::new(create_pkg_with_resources()));
        let mut checker = make_checker(pkg);

        checker.scan_rhai_for_resources("let r = context.instance.resources.app;");
        let findings = checker.finalize();

        assert!(
            !findings.iter().any(|f| f.rule == "hbs/unused-resource"),
            "Resource used in Rhai should not produce warning"
        );
    }

    #[test]
    fn rhai_context_instance_resources_suppresses_unused_resource_warning() {
        let pkg = Box::leak(Box::new(create_pkg_with_resources()));
        let mut checker = make_checker(pkg);

        checker.scan_rhai_for_resources("let r = context.instance.resources.app;");
        let findings = checker.finalize();

        assert!(
            !findings.iter().any(|f| f.rule == "hbs/unused-resource"),
            "Resource used via context.instance.resources in Rhai should not produce warning"
        );
    }

    fn pkg_with_options(options: Vec<(&str, serde_json::Value)>) -> VynilPackageSource {
        let mut map = std::collections::BTreeMap::new();
        for (key, schema) in options {
            map.insert(key.to_string(), schema);
        }
        VynilPackageSource {
            apiVersion: "v1".to_string(),
            kind: "VynilPackage".to_string(),
            metadata: VynilPackageMeta {
                name: "test".to_string(),
                category: "system".to_string(),
                description: "test package".to_string(),
                app_version: None,
                usage: common::vynilpackage::VynilPackageType::Tenant,
                features: vec![],
                backup_affinity: None,
            },
            requirements: vec![],
            recommandations: None,
            options: Some(map),
            images: None,
            resources: None,
            value_script: None,
        }
    }

    fn count_rule(findings: &[LintFinding], rule: &str) -> usize {
        findings.iter().filter(|f| f.rule == rule).count()
    }

    fn option_tree_pkg() -> VynilPackageSource {
        pkg_with_options(vec![
            (
                "ingress",
                serde_json::json!({"type": "object", "properties": {"provider": {"type": "string"}}}),
            ),
            (
                "core",
                serde_json::json!({"type": "object", "properties": {"reloader": {"type": "boolean"}}}),
            ),
            ("dbo", serde_json::json!(true)),
        ])
    }

    #[test]
    fn an_alias_of_context_values_counts_as_usage() {
        let pkg = Box::leak(Box::new(option_tree_pkg()));
        let mut checker = make_checker(pkg);

        checker.scan_rhai_for_values(
            r#"let v = context.values; let a = v.ingress.provider == "traefik"; let b = v.core.reloader;"#,
        );
        let findings = checker.finalize();

        assert!(
            !findings
                .iter()
                .any(|f| f.rule == "hbs/unused-option" && f.message.contains("ingress")),
            "ingress used through a context.values alias should not produce unused-option"
        );
        assert!(
            !findings
                .iter()
                .any(|f| f.rule == "hbs/unused-option-field" && f.message.contains("core.reloader")),
            "core.reloader used through a context.values alias should not produce unused-option-field"
        );
        assert_eq!(
            count_rule(&findings, "hbs/unused-option-field"),
            0,
            "both object leaves are used through the alias"
        );
        assert_eq!(
            count_rule(&findings, "hbs/unused-option"),
            1,
            "only dbo remains unused"
        );
        assert!(
            findings
                .iter()
                .any(|f| f.rule == "hbs/unused-option" && f.message.contains("dbo")),
            "dbo is never used, directly or through the alias"
        );
    }

    #[test]
    fn an_alias_of_a_sub_path_counts_as_usage() {
        let pkg = Box::leak(Box::new(pkg_with_options(vec![(
            "core",
            serde_json::json!({"type": "object", "properties": {"reloader": {"type": "boolean"}}}),
        )])));
        let mut checker = make_checker(pkg);

        checker.scan_rhai_for_values("let c = context.values.core; let b = c.reloader;");
        assert!(
            checker.used_value_paths.contains("core.reloader"),
            "c.reloader should be recorded as context.values.core.reloader"
        );
        let findings = checker.finalize();

        assert!(
            !findings
                .iter()
                .any(|f| f.rule == "hbs/unused-option-field" && f.message.contains("core.reloader")),
            "core.reloader used through a sub-path alias should not be flagged"
        );
        assert!(
            !findings
                .iter()
                .any(|f| f.rule == "hbs/unused-option" && f.message.contains("core")),
            "core is used through its alias"
        );
    }

    #[test]
    fn an_alias_of_an_alias_is_followed() {
        let pkg = Box::leak(Box::new(create_dummy_pkg()));
        let mut checker = make_checker(pkg);

        checker.scan_rhai_for_values("let v = context.values; let i = v.ingress; let p = i.provider;");

        assert!(
            checker.used_values.contains("ingress"),
            "ingress should be recorded as used through the alias chain"
        );
        assert!(
            checker.used_value_paths.contains("ingress.provider"),
            "i.provider should record the joined path ingress.provider"
        );
    }

    #[test]
    fn an_unrelated_variable_is_not_an_alias() {
        let pkg = Box::leak(Box::new(pkg_with_options(vec![("dbo", serde_json::json!(true))])));
        let mut checker = make_checker(pkg);

        checker.scan_rhai_for_values("let v = other.values; let a = v.dbo;");
        assert!(
            !checker.used_values.contains("dbo") && !checker.used_value_paths.contains("dbo"),
            "a variable assigned from another root is not a context.values alias"
        );
        let findings = checker.finalize();

        assert!(
            findings
                .iter()
                .any(|f| f.rule == "hbs/unused-option" && f.message.contains("dbo")),
            "dbo should still produce hbs/unused-option"
        );
    }
}
