use crate::linting::{LintConfig, LintFinding, LintLevel, parse_inline_disables};
use common::{
    rhaihandler::{AST, ASTNode, Engine, Expr, ParseError, Stmt},
    vynilpackage::{VynilPackageSource, VynilPackageType},
};
use rhai::{OptimizationLevel, Position};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

const ENTRY_POINT_PATTERNS: &[&str] = &["install.rhai", "delete.rhai", "reconfigure.rhai"];

// (plural, singular) pairs — k8s_resource must be called with the singular kind
const K8S_RESOURCE_PLURAL_KINDS: &[(&str, &str)] = &[
    ("Deployments", "Deployment"),
    ("Endpoints", "Endpoint"),
    ("Issuers", "Issuer"),
    ("Namespaces", "Namespace"),
    ("Nodes", "Node"),
    ("Secrets", "Secret"),
    ("Services", "Service"),
    ("TenantInstances", "TenantInstance"),
];

const ENTRY_POINT_FUNCTIONS: &[&str] = &["run", "template", "new", "main"];

const FULL_ONLY_FUNCTIONS: &[&str] = &[
    "http_get",
    "http_post",
    "http_put",
    "http_delete",
    "http_patch",
    "get_service_instance",
    "list_service_instances",
    "get_system_instance",
    "list_system_instances",
    "get_tenant_instance",
    "list_tenant_instances",
    "get_jukebox",
    "list_jukeboxes",
    "k8s_resource",
    "k8s_raw",
    "k8s_workload",
];

pub struct RhaiChecker<'a> {
    pkg: &'a VynilPackageSource,
    config: &'a LintConfig,
    resolver_paths: Vec<PathBuf>,
    entry_scripts: HashSet<PathBuf>,
    imported_scripts: HashSet<String>,
    scripts_with_entry_points: HashSet<String>,
    defined_functions: Vec<(String, PathBuf)>,
    called_functions: HashSet<String>,
}

impl<'a> RhaiChecker<'a> {
    pub fn new(
        package_dir: &'a Path,
        config_dir: &'a Path,
        script_dir: Option<&Path>,
        pkg: &'a VynilPackageSource,
        config: &'a LintConfig,
    ) -> Self {
        let mut resolver_paths = vec![package_dir.join("scripts")];
        resolver_paths.push(config_dir.to_path_buf());
        if let Some(dir) = script_dir {
            resolver_paths.push(dir.join("lib"));
            let type_subdir = match pkg.metadata.usage {
                VynilPackageType::Tenant => "tenant",
                VynilPackageType::Service => "service",
                VynilPackageType::System => "system",
            };
            resolver_paths.push(dir.join(type_subdir));
        }

        let mut entry_scripts = HashSet::new();
        let package_scripts_dir = package_dir.join("scripts");
        if package_scripts_dir.is_dir()
            && let Ok(entries) = std::fs::read_dir(&package_scripts_dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "rhai")
                    && let Some(name) = path.file_name().and_then(|n| n.to_str())
                {
                    let is_entry_point = ENTRY_POINT_PATTERNS.contains(&name) || name.starts_with("context_");
                    if !is_entry_point {
                        entry_scripts.insert(path);
                    }
                }
            }
        }

        RhaiChecker {
            pkg,
            config,
            resolver_paths,
            entry_scripts,
            imported_scripts: HashSet::new(),
            scripts_with_entry_points: HashSet::new(),
            defined_functions: Vec::new(),
            called_functions: HashSet::new(),
        }
    }

    pub fn check_file(&mut self, file: &Path, source: &str) -> Vec<LintFinding> {
        let mut findings = Vec::new();

        let mut engine = Engine::new();
        engine.set_max_expr_depths(128, 64);
        engine.set_optimization_level(OptimizationLevel::None);
        match engine.compile(source) {
            Ok(ast) => {
                let import_findings = self.check_imports(&ast, file);
                findings.extend(import_findings);

                let dead_code_findings = check_dead_code(&ast, file);
                findings.extend(dead_code_findings);

                let inline_disables = parse_inline_disables(source);

                let var_findings = check_unused_variables(&ast, file, &inline_disables);
                findings.extend(var_findings);

                let param_findings = check_unused_parameters(&ast, file, self.config, &inline_disables);
                findings.extend(param_findings);

                let empty_catch_findings = check_empty_catch(&ast, file, &inline_disables);
                findings.extend(empty_catch_findings);

                let undef_findings = check_undefined_variables(&ast, file, self.config, &inline_disables);
                findings.extend(undef_findings);

                // Track scripts that define entry-point functions (lifecycle hooks)
                let has_entry_point = ast
                    .iter_fn_def()
                    .any(|f| ENTRY_POINT_FUNCTIONS.contains(&f.name.as_str()));
                if has_entry_point && let Some(stem) = file.file_stem().and_then(|s| s.to_str()) {
                    self.scripts_with_entry_points.insert(stem.to_string());
                }

                let api_mode_findings = check_wrong_api_mode(file, &ast);
                findings.extend(api_mode_findings);

                let pkg_type_findings = check_wrong_package_type(&ast, file, self.pkg);
                findings.extend(pkg_type_findings);

                let plural_findings = check_k8s_resource_plural(&ast, file, self.config, &inline_disables);
                findings.extend(plural_findings);

                let context_findings = check_context_hook_no_return(&ast, file);
                findings.extend(context_findings);

                self.accumulate_functions(&ast, file);
            }
            Err(e) => {
                if let Some(level) =
                    self.config
                        .resolve_level("rhai/syntax", file, LintLevel::Error, &HashSet::new())
                {
                    findings.push(LintFinding {
                        rule: "rhai/syntax".to_string(),
                        level,
                        file: file.to_path_buf(),
                        line: extract_position(&e),
                        message: format!("Syntax error: {e}"),
                    });
                }
            }
        }

        findings
    }

    fn check_imports(&mut self, ast: &AST, file: &Path) -> Vec<LintFinding> {
        let mut findings = Vec::new();

        ast.walk(&mut |nodes: &[ASTNode]| {
            if let Some(node) = nodes.last()
                && let ASTNode::Stmt(stmt) = node
                && let Stmt::Import(import_data, _) = stmt
            {
                let (path_expr, _alias) = &**import_data;
                if let Expr::StringConstant(module_name, _) = path_expr {
                    self.imported_scripts.insert(module_name.to_string());
                    if !self.resolve_import(module_name)
                        && let Some(level) = self.config.resolve_level(
                            "rhai/unresolved-import",
                            file,
                            LintLevel::Error,
                            &HashSet::new(),
                        )
                    {
                        findings.push(LintFinding {
                            rule: "rhai/unresolved-import".to_string(),
                            level,
                            file: file.to_path_buf(),
                            line: None,
                            message: format!("Cannot resolve import \"{module_name}\""),
                        });
                    }
                }
            }
            true
        });

        findings
    }

    fn resolve_import(&self, module_name: &str) -> bool {
        let module_file = format!("{module_name}.rhai");
        for resolver_path in &self.resolver_paths {
            let full_path = resolver_path.join(&module_file);
            if full_path.exists() {
                return true;
            }
        }
        false
    }

    fn accumulate_functions(&mut self, ast: &AST, file: &Path) {
        for fn_def in ast.iter_fn_def() {
            let name = fn_def.name.to_string();
            if !name.starts_with("anon$") {
                self.defined_functions.push((name, file.to_path_buf()));
            }
        }

        ast.walk(&mut |nodes: &[ASTNode]| {
            if let Some(node) = nodes.last()
                && let ASTNode::Expr(expr) = node
            {
                if let Expr::FnCall(fn_call, _) = expr {
                    self.called_functions.insert(fn_call.name.to_string());
                } else if let Expr::MethodCall(fn_call, _) = expr {
                    self.called_functions.insert(fn_call.name.to_string());
                }
            }
            true
        });
    }

    pub fn finalize(&self) -> Vec<LintFinding> {
        let mut findings = Vec::new();

        // Check unused-script: skip scripts that define entry-point functions (lifecycle hooks)
        for script_path in &self.entry_scripts {
            if let Some(file_stem) = script_path.file_stem().and_then(|s| s.to_str())
                && !self.imported_scripts.contains(file_stem)
                && !self.scripts_with_entry_points.contains(file_stem)
                && let Some(level) = self.config.resolve_level(
                    "rhai/unused-script",
                    script_path,
                    LintLevel::Warn,
                    &HashSet::new(),
                )
            {
                findings.push(LintFinding {
                    rule: "rhai/unused-script".to_string(),
                    level,
                    file: script_path.clone(),
                    line: None,
                    message: format!("Script `{file_stem}` defined but never imported"),
                });
            }
        }

        // Check unused-function
        for (func_name, func_file) in &self.defined_functions {
            if !self.called_functions.contains(func_name)
                && !ENTRY_POINT_FUNCTIONS.contains(&func_name.as_str())
                && let Some(level) = self.config.resolve_level(
                    "rhai/unused-function",
                    func_file,
                    LintLevel::Warn,
                    &HashSet::new(),
                )
            {
                findings.push(LintFinding {
                    rule: "rhai/unused-function".to_string(),
                    level,
                    file: func_file.clone(),
                    line: None,
                    message: format!("Function `{func_name}` defined but never called"),
                });
            }
        }

        findings
    }
}

/// Recursively collects variable names used in an expression.
/// Unlike `ast.walk()`, this correctly descends into method call arguments
/// (rhai 1.20.0 `ast.walk()` skips them).
fn collect_used_vars_expr(expr: &Expr, vars: &mut HashSet<String>) {
    match expr {
        Expr::Variable(var_data, _, _) => {
            let (_, name, _, _) = &**var_data;
            vars.insert(name.to_string());
        }
        Expr::Dot(data, _, _) | Expr::Index(data, _, _) => {
            collect_used_vars_expr(&data.lhs, vars);
            collect_used_vars_expr(&data.rhs, vars);
        }
        Expr::FnCall(fn_call, _) | Expr::MethodCall(fn_call, _) => {
            for arg in &fn_call.args {
                collect_used_vars_expr(arg, vars);
            }
        }
        Expr::Array(items, _) => {
            for item in items {
                collect_used_vars_expr(item, vars);
            }
        }
        Expr::Map(pairs, _) => {
            for (_, val) in &pairs.0 {
                collect_used_vars_expr(val, vars);
            }
        }
        Expr::InterpolatedString(parts, _) => {
            for part in parts {
                collect_used_vars_expr(part, vars);
            }
        }
        Expr::And(data, _) | Expr::Or(data, _) | Expr::Coalesce(data, _) => {
            if let Some(left) = data.first() {
                collect_used_vars_expr(left, vars);
            }
            if let Some(right) = data.get(1) {
                collect_used_vars_expr(right, vars);
            }
        }
        // `if`/`while`/etc. used as expressions are wrapped in Expr::Stmt
        Expr::Stmt(block) => {
            collect_used_vars_stmts(block.statements(), vars);
        }
        _ => {}
    }
}

fn collect_used_vars_stmts(stmts: &[Stmt], vars: &mut HashSet<String>) {
    for stmt in stmts {
        collect_used_vars_stmt(stmt, vars);
    }
}

fn collect_used_vars_stmt(stmt: &Stmt, vars: &mut HashSet<String>) {
    match stmt {
        Stmt::Var(var_data, _, _) => {
            let (_, init, _) = &**var_data;
            collect_used_vars_expr(init, vars);
        }
        Stmt::Expr(expr) => collect_used_vars_expr(expr, vars),
        Stmt::Return(Some(e), ..) => {
            collect_used_vars_expr(e, vars);
        }
        Stmt::If(data, _) => {
            collect_used_vars_expr(&data.expr, vars);
            collect_used_vars_stmts(data.body.statements(), vars);
            collect_used_vars_stmts(data.branch.statements(), vars);
        }
        Stmt::While(data, _) | Stmt::Do(data, ..) => {
            collect_used_vars_expr(&data.expr, vars);
            collect_used_vars_stmts(data.body.statements(), vars);
        }
        Stmt::For(data, _) => {
            collect_used_vars_expr(&data.2.expr, vars);
            collect_used_vars_stmts(data.2.body.statements(), vars);
        }
        Stmt::Block(block) => collect_used_vars_stmts(block.statements(), vars),
        Stmt::TryCatch(data, _) => {
            collect_used_vars_stmts(data.body.statements(), vars);
            collect_used_vars_stmts(data.branch.statements(), vars);
        }
        Stmt::Assignment(data) => {
            collect_used_vars_expr(&data.1.lhs, vars);
            collect_used_vars_expr(&data.1.rhs, vars);
        }
        // Top-level operator expressions (e.g. `a >= b`) are Stmt::FnCall, not Stmt::Expr
        Stmt::FnCall(fn_call, _) => {
            for arg in &fn_call.args {
                collect_used_vars_expr(arg, vars);
            }
        }
        _ => {}
    }
}

/// Scope-aware shadowing check: walks statements maintaining a scope stack.
/// Only flags shadowing when a name is already declared in the current scope chain,
/// preventing false positives for sibling scopes (e.g. two try blocks).
fn check_shadowing_scoped(
    stmts: &[Stmt],
    file: &Path,
    scope_stack: &mut Vec<HashSet<String>>,
    findings: &mut Vec<LintFinding>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::Var(var_data, _, pos) => {
                let (ident, _, _) = &**var_data;
                let name = ident.name.to_string();
                if !name.starts_with('_') {
                    if scope_stack.iter().any(|s| s.contains(&name)) {
                        findings.push(LintFinding {
                            rule: "rhai/shadowed-variable".to_string(),
                            level: LintLevel::Warn,
                            file: file.to_path_buf(),
                            line: Some(pos.line().unwrap_or(0)),
                            message: format!("Variable `{name}` shadows a previous declaration"),
                        });
                    }
                    if let Some(scope) = scope_stack.last_mut() {
                        scope.insert(name);
                    }
                }
            }
            Stmt::Block(block) => {
                scope_stack.push(HashSet::new());
                check_shadowing_scoped(block.statements(), file, scope_stack, findings);
                scope_stack.pop();
            }
            Stmt::If(data, _) | Stmt::TryCatch(data, _) => {
                scope_stack.push(HashSet::new());
                check_shadowing_scoped(data.body.statements(), file, scope_stack, findings);
                scope_stack.pop();
                scope_stack.push(HashSet::new());
                check_shadowing_scoped(data.branch.statements(), file, scope_stack, findings);
                scope_stack.pop();
            }
            Stmt::While(data, _) | Stmt::Do(data, ..) => {
                scope_stack.push(HashSet::new());
                check_shadowing_scoped(data.body.statements(), file, scope_stack, findings);
                scope_stack.pop();
            }
            Stmt::For(data, _) => {
                scope_stack.push(HashSet::new());
                if let Some(top) = scope_stack.last_mut() {
                    top.insert(data.0.name.to_string());
                }
                check_shadowing_scoped(data.2.body.statements(), file, scope_stack, findings);
                scope_stack.pop();
            }
            _ => {}
        }
    }
}

fn check_dead_code(ast: &AST, file: &Path) -> Vec<LintFinding> {
    let mut findings = Vec::new();
    check_statements_for_dead_code(ast.statements(), file, &mut findings);
    findings
}

fn check_statements_for_dead_code(statements: &[Stmt], file: &Path, findings: &mut Vec<LintFinding>) {
    for (idx, stmt) in statements.iter().enumerate() {
        if matches!(stmt, Stmt::Return(..) | Stmt::BreakLoop(..)) {
            for _dead_idx in idx.saturating_add(1)..statements.len() {
                findings.push(LintFinding {
                    rule: "rhai/dead-code".to_string(),
                    level: LintLevel::Warn,
                    file: file.to_path_buf(),
                    line: None,
                    message: "Unreachable code after return or break".to_string(),
                });
            }
            break;
        }

        match stmt {
            Stmt::Block(block) => {
                check_statements_for_dead_code(block.statements(), file, findings);
            }
            Stmt::If(flow_control, ..) | Stmt::TryCatch(flow_control, ..) => {
                check_statements_for_dead_code(flow_control.body.statements(), file, findings);
                check_statements_for_dead_code(flow_control.branch.statements(), file, findings);
            }
            Stmt::While(flow_control, ..) | Stmt::Do(flow_control, ..) => {
                check_statements_for_dead_code(flow_control.body.statements(), file, findings);
            }
            Stmt::For(data, ..) => {
                check_statements_for_dead_code(data.2.body.statements(), file, findings);
            }
            _ => {}
        }
    }
}

fn collect_declared_vars_stmts(stmts: &[Stmt], declared: &mut HashSet<String>) {
    for stmt in stmts {
        collect_declared_vars_stmt(stmt, declared);
    }
}

fn collect_declared_vars_stmt(stmt: &Stmt, declared: &mut HashSet<String>) {
    match stmt {
        Stmt::Var(var_data, _, _) => {
            let (ident, _, _) = &**var_data;
            declared.insert(ident.name.to_string());
        }
        Stmt::Block(block) => collect_declared_vars_stmts(block.statements(), declared),
        Stmt::If(data, _) => {
            collect_declared_vars_stmts(data.body.statements(), declared);
            collect_declared_vars_stmts(data.branch.statements(), declared);
        }
        Stmt::While(data, _) | Stmt::Do(data, ..) => {
            collect_declared_vars_stmts(data.body.statements(), declared);
        }
        Stmt::For(data, _) => {
            declared.insert(data.0.name.to_string());
            collect_declared_vars_stmts(data.2.body.statements(), declared);
        }
        Stmt::TryCatch(data, _) => {
            // Catch variable (e.g. `e` in `catch(e)`) is bound in the catch block
            if let Expr::Variable(var_data, _, _) = &data.expr {
                let (_, name, _, _) = &**var_data;
                declared.insert(name.to_string());
            }
            collect_declared_vars_stmts(data.body.statements(), declared);
            collect_declared_vars_stmts(data.branch.statements(), declared);
        }
        _ => {}
    }
}

fn check_undefined_variables(
    ast: &AST,
    file: &Path,
    config: &LintConfig,
    inline_disables: &HashMap<usize, HashSet<String>>,
) -> Vec<LintFinding> {
    let mut findings = Vec::new();

    let file_disabled: HashSet<String> = inline_disables
        .values()
        .flat_map(|rules| rules.iter().cloned())
        .collect();

    for fn_def in ast.iter_fn_def() {
        if fn_def.name.starts_with("anon$") {
            continue;
        }

        let mut known: HashSet<String> = fn_def
            .params
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
        collect_declared_vars_stmts(fn_def.body.statements(), &mut known);

        let mut refs = HashSet::new();
        collect_used_vars_stmts(fn_def.body.statements(), &mut refs);

        let mut flagged: Vec<String> = refs
            .into_iter()
            .filter(|name| !known.contains(name) && !name.starts_with('_'))
            .collect();
        flagged.sort();

        for name in flagged {
            if let Some(level) =
                config.resolve_level("rhai/undefined-variable", file, LintLevel::Error, &file_disabled)
            {
                findings.push(LintFinding {
                    rule: "rhai/undefined-variable".to_string(),
                    level,
                    file: file.to_path_buf(),
                    line: None,
                    message: format!("Variable `{name}` is used but never declared"),
                });
            }
        }
    }

    findings
}

fn check_empty_catch(
    ast: &AST,
    file: &Path,
    inline_disables: &HashMap<usize, HashSet<String>>,
) -> Vec<LintFinding> {
    let mut findings = Vec::new();

    // ast.walk() finds TryCatch nodes in top-level statements and function bodies,
    // but does NOT recurse into try/catch bodies itself. We handle nested try-catch
    // by delegating recursion to check_stmts_for_empty_catch when a TryCatch is found.
    ast.walk(&mut |nodes: &[ASTNode]| {
        if let Some(ASTNode::Stmt(Stmt::TryCatch(data, pos))) = nodes.last() {
            if data.branch.statements().is_empty() {
                let line = pos.line().unwrap_or(0);
                let disabled = empty_catch_disabled(line, data.branch.end_position(), inline_disables);
                if !disabled {
                    findings.push(LintFinding {
                        rule: "rhai/empty-catch".to_string(),
                        level: LintLevel::Warn,
                        file: file.to_path_buf(),
                        line: Some(line),
                        message: "Empty catch block silently ignores errors".to_string(),
                    });
                }
            }
            // Manually recurse into try/catch bodies to catch nested try-catch
            check_stmts_for_empty_catch(data.body.statements(), file, inline_disables, &mut findings);
            check_stmts_for_empty_catch(data.branch.statements(), file, inline_disables, &mut findings);
        }
        true
    });

    findings
}

/// Vrai quand une directive `vynil-lint-disable rhai/empty-catch` se trouve sur une ligne du
/// bloc `try { } catch { }`, du `try` (où le constat est rapporté) à la fin du `catch`.
fn empty_catch_disabled(
    try_line: usize,
    catch_end: Position,
    inline_disables: &HashMap<usize, HashSet<String>>,
) -> bool {
    let last = catch_end.line().unwrap_or(try_line).max(try_line);
    (try_line..=last).any(|line| {
        inline_disables
            .get(&line)
            .is_some_and(|rules| rules.contains("rhai/empty-catch"))
    })
}

fn check_stmts_for_empty_catch(
    stmts: &[Stmt],
    file: &Path,
    inline_disables: &HashMap<usize, HashSet<String>>,
    findings: &mut Vec<LintFinding>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::TryCatch(data, pos) => {
                if data.branch.statements().is_empty() {
                    let line = pos.line().unwrap_or(0);
                    let disabled = empty_catch_disabled(line, data.branch.end_position(), inline_disables);
                    if !disabled {
                        findings.push(LintFinding {
                            rule: "rhai/empty-catch".to_string(),
                            level: LintLevel::Warn,
                            file: file.to_path_buf(),
                            line: Some(line),
                            message: "Empty catch block silently ignores errors".to_string(),
                        });
                    }
                }
                check_stmts_for_empty_catch(data.body.statements(), file, inline_disables, findings);
                check_stmts_for_empty_catch(data.branch.statements(), file, inline_disables, findings);
            }
            Stmt::Block(block) => {
                check_stmts_for_empty_catch(block.statements(), file, inline_disables, findings);
            }
            Stmt::If(flow_control, _) => {
                check_stmts_for_empty_catch(flow_control.body.statements(), file, inline_disables, findings);
                check_stmts_for_empty_catch(
                    flow_control.branch.statements(),
                    file,
                    inline_disables,
                    findings,
                );
            }
            Stmt::While(data, _) | Stmt::Do(data, ..) => {
                check_stmts_for_empty_catch(data.body.statements(), file, inline_disables, findings);
            }
            Stmt::For(data, _) => {
                check_stmts_for_empty_catch(data.2.body.statements(), file, inline_disables, findings);
            }
            _ => {}
        }
    }
}

const fn extract_position(e: &ParseError) -> Option<usize> {
    e.position().line()
}

fn check_unused_variables(
    ast: &AST,
    file: &Path,
    inline_disables: &HashMap<usize, HashSet<String>>,
) -> Vec<LintFinding> {
    let mut findings = Vec::new();
    let mut declarations: Vec<(String, usize)> = Vec::new();
    let mut used: HashSet<String> = HashSet::new();

    // Collect variable declarations
    ast.walk(&mut |nodes: &[ASTNode]| {
        if let Some(ASTNode::Stmt(Stmt::Var(var_data, _, pos))) = nodes.last() {
            let (ident, _, _) = &**var_data;
            declarations.push((ident.name.to_string(), pos.line().unwrap_or(0)));
        }
        true
    });

    // Collect variable usages — use explicit traversal instead of ast.walk() because
    // walk() does not descend into try-catch blocks, causing false positives for
    // variables used only inside try { } bodies.
    collect_used_vars_stmts(ast.statements(), &mut used);
    for fn_def in ast.iter_fn_def() {
        collect_used_vars_stmts(fn_def.body.statements(), &mut used);
    }

    // Check for shadowing (scope-aware: sibling try/if/for blocks don't count)
    {
        let mut scope: Vec<HashSet<String>> = vec![HashSet::new()];
        check_shadowing_scoped(ast.statements(), file, &mut scope, &mut findings);
        for fn_def in ast.iter_fn_def() {
            let params: HashSet<String> = fn_def
                .params
                .iter()
                .map(std::string::ToString::to_string)
                .collect();
            let mut fn_scope: Vec<HashSet<String>> = vec![params];
            check_shadowing_scoped(fn_def.body.statements(), file, &mut fn_scope, &mut findings);
        }
    }

    // Check for unused variables
    for (name, line) in declarations {
        if !name.starts_with('_') && !used.contains(&name) {
            let disabled = inline_disables
                .get(&line)
                .is_some_and(|rules| rules.contains("rhai/unused-variable"));
            if !disabled {
                findings.push(LintFinding {
                    rule: "rhai/unused-variable".to_string(),
                    level: LintLevel::Warn,
                    file: file.to_path_buf(),
                    line: Some(line),
                    message: format!("Variable `{name}` is declared but never used"),
                });
            }
        }
    }

    findings
}

fn check_unused_parameters(
    ast: &AST,
    file: &Path,
    config: &LintConfig,
    inline_disables: &HashMap<usize, HashSet<String>>,
) -> Vec<LintFinding> {
    // Aggregate all inline-disabled rules across the file for file-level suppression
    let file_disabled: HashSet<String> = inline_disables
        .values()
        .flat_map(|rules| rules.iter().cloned())
        .collect();

    let mut findings = Vec::new();

    for fn_def in ast.iter_fn_def() {
        let mut used_params: HashSet<String> = HashSet::new();
        collect_used_vars_stmts(fn_def.body.statements(), &mut used_params);

        let fn_line = fn_def.body.position().line();
        for param_name in &fn_def.params {
            let param_str = param_name.to_string();
            if !param_str.starts_with('_')
                && !used_params.contains(&param_str)
                && let Some(level) =
                    config.resolve_level("rhai/unused-parameter", file, LintLevel::Warn, &file_disabled)
            {
                findings.push(LintFinding {
                    rule: "rhai/unused-parameter".to_string(),
                    level,
                    file: file.to_path_buf(),
                    line: fn_line,
                    message: format!(
                        "Parameter `{}` in function `{}` is never used",
                        param_str, fn_def.name
                    ),
                });
            }
        }
    }

    findings
}

fn is_core_script(file: &Path) -> bool {
    if let Some(file_name) = file.file_name().and_then(|n| n.to_str())
        && (file_name == "build.rhai" || file_name == "validate.rhai")
    {
        return true;
    }
    file.to_string_lossy().contains("handlebars/helpers/")
}

fn check_wrong_api_mode(file: &Path, ast: &AST) -> Vec<LintFinding> {
    let mut findings = Vec::new();

    if !is_core_script(file) {
        return findings;
    }

    ast.walk(&mut |nodes: &[ASTNode]| {
        if let Some(ASTNode::Expr(Expr::FnCall(fn_call, _))) = nodes.last() {
            let fn_name = fn_call.name.to_string();
            if FULL_ONLY_FUNCTIONS.contains(&fn_name.as_str()) {
                findings.push(LintFinding {
                    rule: "rhai/wrong-api-mode".to_string(),
                    level: LintLevel::Warn,
                    file: file.to_path_buf(),
                    line: None,
                    message: format!("Function `{fn_name}` is not available in core mode scripts"),
                });
            }
        }
        true
    });

    findings
}

/// Extract the leftmost property name from a Rhai dot-chain RHS.
///
/// Rhai compiles `a.b.c` right-associatively as `Dot(Variable(a), Dot(Property(b), Property(c)))`,
/// so the first property accessed on `a` is the `lhs` of the nested `Dot`, not a direct child.
fn first_property_in_dot_chain(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Property(prop_data, _) => Some(&prop_data.2),
        Expr::Dot(inner_data, _, _) => first_property_in_dot_chain(&inner_data.lhs),
        _ => None,
    }
}

fn check_wrong_package_type(ast: &AST, file: &Path, pkg: &VynilPackageSource) -> Vec<LintFinding> {
    let mut findings = Vec::new();

    if pkg.metadata.usage != VynilPackageType::System {
        return findings;
    }

    // Context builder hooks in files/ are explicitly allowed to work with tenant data
    if file.starts_with("files/") {
        return findings;
    }

    ast.walk(&mut |nodes: &[ASTNode]| {
        if let Some(ASTNode::Expr(expr)) = nodes.last() {
            match expr {
                Expr::Variable(var_data, _, pos) => {
                    let (_, name, _, _) = &**var_data;
                    if *name == "tenant" {
                        findings.push(LintFinding {
                            rule: "rhai/wrong-package-type".to_string(),
                            level: LintLevel::Warn,
                            file: file.to_path_buf(),
                            line: pos.line(),
                            message: "System packages cannot access tenant context".to_string(),
                        });
                    }
                }
                Expr::Dot(data, _, _) => {
                    if let Expr::Variable(var_data, _, _) = &data.lhs {
                        let (_, var_name, _, _) = &**var_data;
                        if *var_name == "context" {
                            // In Rhai's AST, `context.namespace.ha` is right-associative:
                            // Dot(Variable("context"), Dot(Property("namespace"), Property("ha")))
                            // So the first property is the leftmost node of the rhs chain.
                            if let Some(prop) = first_property_in_dot_chain(&data.rhs)
                                && (prop == "namespace" || prop == "tenant")
                            {
                                findings.push(LintFinding {
                                    rule: "rhai/wrong-package-type".to_string(),
                                    level: LintLevel::Warn,
                                    file: file.to_path_buf(),
                                    line: data.rhs.position().line(),
                                    message: format!(
                                        "System packages cannot access context.{prop} (not available for type:system)",
                                    ),
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        true
    });

    findings
}

fn check_context_hook_no_return(ast: &AST, file: &Path) -> Vec<LintFinding> {
    let mut findings = Vec::new();

    if !file.to_string_lossy().contains("context_") {
        return findings;
    }

    let mut has_valid_return = false;

    for fn_def in ast.iter_fn_def() {
        let stmts = fn_def.body.iter().collect::<Vec<_>>();
        if let Some(Stmt::Return(..) | Stmt::Expr(..)) = stmts.last() {
            has_valid_return = true;
            break;
        }
    }

    if !has_valid_return {
        let stmts = ast.statements();
        if let Some(Stmt::Return(..) | Stmt::Expr(..)) = stmts.last() {
            has_valid_return = true;
        }
    }

    if !has_valid_return {
        findings.push(LintFinding {
            rule: "rhai/context-hook-no-return".to_string(),
            level: LintLevel::Error,
            file: file.to_path_buf(),
            line: None,
            message: "Context hook must return the context".to_string(),
        });
    }

    findings
}

fn check_k8s_resource_plural(
    ast: &AST,
    file: &Path,
    config: &LintConfig,
    inline_disables: &HashMap<usize, HashSet<String>>,
) -> Vec<LintFinding> {
    let mut findings = Vec::new();

    ast.walk(&mut |nodes: &[ASTNode]| {
        if let Some(ASTNode::Expr(Expr::FnCall(fn_call, _))) = nodes.last()
            && fn_call.name.as_str() == "k8s_resource"
            && let Some(Expr::StringConstant(kind, pos)) = fn_call.args.first()
            && let Some((_, singular)) = K8S_RESOURCE_PLURAL_KINDS
                .iter()
                .find(|(plural, _)| *plural == kind.as_str())
        {
            let line = pos.line();
            let disabled = line
                .and_then(|l| inline_disables.get(&l))
                .cloned()
                .unwrap_or_default();
            if let Some(level) =
                config.resolve_level("rhai/k8s-resource-plural", file, LintLevel::Warn, &disabled)
            {
                findings.push(LintFinding {
                    rule: "rhai/k8s-resource-plural".to_string(),
                    level,
                    file: file.to_path_buf(),
                    line,
                    message: format!(
                        "`k8s_resource(\"{kind}\", ...)` uses plural kind; use \"{singular}\" instead"
                    ),
                });
            }
        }
        true
    });

    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::vynilpackage::read_package_yaml;

    fn get_fixture_dir(name: &str) -> PathBuf {
        let base = env!("CARGO_MANIFEST_DIR");
        PathBuf::from(format!("{base}/tests/fixtures/lint/{name}/"))
    }

    #[test]
    fn rhai_syntax_error_detected() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "let x = ;";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            findings.iter().any(|f| f.rule == "rhai/syntax"),
            "Expected rhai/syntax finding"
        );
    }

    #[test]
    fn valid_rhai_no_syntax_error() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "let x = 1;";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/syntax"),
            "Expected no rhai/syntax finding"
        );
    }

    #[test]
    fn resolved_import_no_error() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = r#"import "mylib" as lib;"#;
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/unresolved-import"),
            "Expected no rhai/unresolved-import finding"
        );
    }

    #[test]
    fn unresolved_import_produces_error() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = r#"import "nonexistent" as x;"#;
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            findings.iter().any(|f| f.rule == "rhai/unresolved-import"),
            "Expected rhai/unresolved-import finding"
        );
    }

    #[test]
    fn unresolved_import_no_duplicate() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = r#"import "nonexistent" as x;"#;
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        let count = findings
            .iter()
            .filter(|f| f.rule == "rhai/unresolved-import")
            .count();
        assert_eq!(count, 1, "Expected exactly 1 unresolved-import, got {count}");
    }

    #[test]
    #[ignore = "dead code detection in function bodies requires private AST API access"]
    fn dead_code_after_return_detected() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "fn foo() { return 1; let x = 2; }";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            findings.iter().any(|f| f.rule == "rhai/dead-code"),
            "Expected rhai/dead-code finding"
        );
    }

    #[test]
    fn unused_script_produces_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let install_source = "fn run() {}";
        let _ = checker.check_file(&PathBuf::from("scripts/install.rhai"), install_source);

        let findings = checker.finalize();

        assert!(
            findings
                .iter()
                .any(|f| f.rule == "rhai/unused-script" && f.message.contains("orphan")),
            "Expected rhai/unused-script finding for orphan"
        );
    }

    #[test]
    fn lifecycle_hook_script_not_flagged_as_unused() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        // Simulate a lifecycle hook script (has a run function, never imported)
        let hook_source = "fn run(instance, context) { context }";
        let _ = checker.check_file(&PathBuf::from("scripts/install_post.rhai"), hook_source);

        let findings = checker.finalize();

        assert!(
            !findings
                .iter()
                .any(|f| f.rule == "rhai/unused-script" && f.message.contains("install_post")),
            "Lifecycle hook with run() should not be flagged as unused"
        );
    }

    #[test]
    fn used_variable_no_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "let x = 1; x + 1;";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/unused-variable"),
            "Expected no rhai/unused-variable finding"
        );
    }

    #[test]
    fn unused_variable_produces_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "let unused = 42;";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            findings
                .iter()
                .any(|f| f.rule == "rhai/unused-variable" && f.message.contains("unused")),
            "Expected rhai/unused-variable finding for 'unused'"
        );
    }

    #[test]
    fn variable_used_in_assignment_rhs_no_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        // `ing` used as RHS of a property assignment — must not be flagged
        let source = r#"
            let obj = #{};
            let ing = #{spec: #{rules: [#{host: "h"}]}};
            obj.host = ing.spec.rules[0].host;
        "#;
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings
                .iter()
                .any(|f| f.rule == "rhai/unused-variable" && f.message.contains("`ing`")),
            "Variable used in assignment RHS should not be flagged as unused"
        );
    }

    #[test]
    fn variable_used_in_assignment_lhs_no_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        // `obj` used as LHS target of an assignment — must not be flagged
        let source = "let obj = #{}; obj.x = 1;";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings
                .iter()
                .any(|f| f.rule == "rhai/unused-variable" && f.message.contains("`obj`")),
            "Variable used as assignment LHS should not be flagged as unused"
        );
    }

    #[test]
    fn underscore_variable_ignored() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "let _unused = 42;";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/unused-variable"),
            "Expected no rhai/unused-variable finding for underscore-prefixed variables"
        );
    }

    #[test]
    fn shadowed_variable_produces_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "let x = 1; let x = 2;";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            findings.iter().any(|f| f.rule == "rhai/shadowed-variable"),
            "Expected rhai/shadowed-variable finding"
        );
    }

    #[test]
    fn shadowed_variable_no_duplicate() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "let x = 1; let x = 2;";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        let count = findings
            .iter()
            .filter(|f| f.rule == "rhai/shadowed-variable")
            .count();
        assert_eq!(
            count, 1,
            "Expected exactly 1 shadowed-variable warning, got {count}"
        );
    }

    #[test]
    fn unused_parameter_produces_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "fn foo(used, unused) { used }";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            findings
                .iter()
                .any(|f| f.rule == "rhai/unused-parameter" && f.message.contains("unused")),
            "Expected rhai/unused-parameter finding for 'unused'"
        );
    }

    #[test]
    fn unused_function_produces_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "fn helper() { 42 }";
        let _ = checker.check_file(&PathBuf::from("test.rhai"), source);

        let findings = checker.finalize();

        assert!(
            findings
                .iter()
                .any(|f| f.rule == "rhai/unused-function" && f.message.contains("helper")),
            "Expected rhai/unused-function finding for 'helper'"
        );
    }

    #[test]
    fn run_function_not_flagged_as_unused() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "fn run(inst, ctx, args) { 42 }";
        let _ = checker.check_file(&PathBuf::from("test.rhai"), source);

        let findings = checker.finalize();

        assert!(
            !findings
                .iter()
                .any(|f| f.rule == "rhai/unused-function" && f.message.contains("run")),
            "Expected no rhai/unused-function finding for 'run' entry point"
        );
    }

    #[test]
    fn full_api_in_core_script_warns() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = r#"let r = k8s_resource("Pod");"#;
        let findings = checker.check_file(&PathBuf::from("handlebars/helpers/test.rhai"), source);

        assert!(
            findings.iter().any(|f| f.rule == "rhai/wrong-api-mode"),
            "Expected rhai/wrong-api-mode finding for k8s_resource in core script"
        );
    }

    #[test]
    fn full_api_in_lifecycle_script_ok() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = r#"let r = k8s_resource("Pod");"#;
        let findings = checker.check_file(&PathBuf::from("scripts/install.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/wrong-api-mode"),
            "Expected no rhai/wrong-api-mode finding for k8s_resource in lifecycle script"
        );
    }

    #[test]
    fn context_hook_with_return_ok() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "fn run(inst, ctx, args) { ctx.extra = 1; return ctx; }";
        let findings = checker.check_file(&PathBuf::from("scripts/context_post.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/context-hook-no-return"),
            "Expected no rhai/context-hook-no-return finding for valid context hook"
        );
    }

    #[test]
    fn context_hook_without_return_errors() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "let x = 1;";
        let findings = checker.check_file(&PathBuf::from("scripts/context_post.rhai"), source);

        assert!(
            findings.iter().any(|f| f.rule == "rhai/context-hook-no-return"),
            "Expected rhai/context-hook-no-return finding for context hook without return"
        );
    }

    #[test]
    fn tenant_access_in_system_package_warns() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "let name = tenant.name;";
        let findings = checker.check_file(&PathBuf::from("scripts/tenant_in_system.rhai"), source);

        assert!(
            findings.iter().any(|f| f.rule == "rhai/wrong-package-type"),
            "Expected rhai/wrong-package-type finding for tenant access"
        );
    }

    #[test]
    fn parameter_used_in_method_call_arg_no_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        // context is used only as a method call argument — rhai 1.20+ ast.walk() missed these
        let source = r"fn run(instance, context) {
  instance.set_services([#{ key: context.ns }]);
}";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings
                .iter()
                .any(|f| f.rule == "rhai/unused-parameter" && f.message.contains("context")),
            "context used as method call arg should not be flagged as unused"
        );
    }

    #[test]
    fn underscore_parameter_no_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "fn run(instance, _context) { instance.foo(); }";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/unused-parameter"),
            "_context should not be flagged as unused parameter"
        );
    }

    #[test]
    fn sibling_try_blocks_no_shadowing_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        // Two `let api` in sibling try blocks — not true shadowing
        let source = r"fn run(ctx) {
  try { let api = ctx.a; api.get(); } catch {}
  try { let api = ctx.b; api.get(); } catch {}
}";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/shadowed-variable"),
            "Same variable name in sibling try blocks should not be flagged as shadowing"
        );
    }

    #[test]
    fn nested_block_shadowing_still_warns() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        // let x declared in outer scope then re-declared in inner block — real shadowing
        let source = "fn run() { let x = 1; if true { let x = 2; x } }";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            findings.iter().any(|f| f.rule == "rhai/shadowed-variable"),
            "Variable re-declared in nested scope should still be flagged"
        );
    }

    #[test]
    fn parameter_used_in_if_expr_no_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        // context is used inside an if-expression used as init value (Expr::Stmt case)
        let source = r"fn template(_instance, context) {
    let replicas = if context.cluster.ha { 2 } else { 1 };
    replicas
}";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings
                .iter()
                .any(|f| f.rule == "rhai/unused-parameter" && f.message.contains("context")),
            "context used in if-expression init should not be flagged as unused"
        );
    }

    #[test]
    fn parameter_used_in_closure_operator_no_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        // v and app_version are used inside a closure body that is a Stmt::FnCall (operator)
        let source = r#"fn run(args) {
    let app_version = "1.0.0";
    let more = [1,2,3].filter(|v| v >= 2); // vynil-lint-disable rhai/unused-parameter
    more
}"#;
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings
                .iter()
                .any(|f| f.rule == "rhai/unused-parameter" && f.message.contains("\"v\"")),
            "v used in closure operator body should not be flagged as unused"
        );
    }

    #[test]
    fn inline_disable_suppresses_unused_parameter() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "fn foo(used, unused) { used } // vynil-lint-disable rhai/unused-parameter";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/unused-parameter"),
            "inline disable should suppress rhai/unused-parameter"
        );
    }

    #[test]
    fn inline_disable_suppresses_unused_variable() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "let x = 1; // vynil-lint-disable rhai/unused-variable\nlet used = 2;\nused";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/unused-variable"),
            "inline disable should suppress rhai/unused-variable"
        );
    }

    #[test]
    fn variable_used_in_try_block_no_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "let x = 1;\ntry { let _r = x + 1; } catch {}";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings
                .iter()
                .any(|f| f.rule == "rhai/unused-variable" && f.message.contains("`x`")),
            "variable used inside try block should not be flagged as unused"
        );
    }

    #[test]
    fn variable_used_via_dot_access_no_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "fn run(ctx) {\n  let obj = ctx.values;\n  obj.name\n}";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings
                .iter()
                .any(|f| f.rule == "rhai/unused-variable" && f.message.contains("`obj`")),
            "variable used as dot-access base should not be flagged as unused"
        );
    }

    #[test]
    fn empty_catch_warns() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "try { let _x = 1; } catch {}";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            findings.iter().any(|f| f.rule == "rhai/empty-catch"),
            "empty catch block should be flagged as rhai/empty-catch"
        );
    }

    #[test]
    fn non_empty_catch_no_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = r"try { let _x = 1; } catch(e) { print(e); }";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/empty-catch"),
            "non-empty catch block should not be flagged"
        );
    }

    #[test]
    fn undefined_variable_in_function_produces_error() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "fn run(instance, context) { log_info(con_bug_text.toto); }";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            findings
                .iter()
                .any(|f| f.rule == "rhai/undefined-variable" && f.message.contains("con_bug_text")),
            "Expected rhai/undefined-variable for 'con_bug_text'"
        );
    }

    #[test]
    fn function_params_not_flagged_as_undefined() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "fn run(instance, context) { log_info(context.field); }";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/undefined-variable"),
            "Function parameters should not be flagged as undefined"
        );
    }

    #[test]
    fn let_variable_not_flagged_as_undefined() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "fn run(instance, context) { let x = context.field; log_info(x.foo); }";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/undefined-variable"),
            "Let-declared variables should not be flagged as undefined"
        );
    }

    #[test]
    fn catch_variable_not_flagged_as_undefined() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = r"fn run(instance, context) { try { log_info(context.x); } catch(e) { log_warn(e); } }";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings
                .iter()
                .any(|f| f.rule == "rhai/undefined-variable" && f.message.contains("`e`")),
            "Catch-bound variable should not be flagged as undefined"
        );
    }

    #[test]
    fn inline_disable_suppresses_undefined_variable() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "fn run(instance, context) { log_info(con_bug_text.toto); } // vynil-lint-disable rhai/undefined-variable";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/undefined-variable"),
            "inline disable should suppress rhai/undefined-variable"
        );
    }

    #[test]
    fn inline_disable_suppresses_empty_catch() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "try { let _x = 1; } catch {} // vynil-lint-disable rhai/empty-catch";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/empty-catch"),
            "inline disable should suppress rhai/empty-catch"
        );
    }

    #[test]
    fn inline_disable_on_the_catch_line_suppresses_empty_catch() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "let x = 0;\ntry {\n    x = 1;\n} catch {} // vynil-lint-disable rhai/empty-catch\n";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/empty-catch"),
            "la directive sur la ligne du catch doit supprimer rhai/empty-catch : {findings:?}"
        );
    }

    #[test]
    fn inline_disable_inside_the_try_block_suppresses_empty_catch() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = "let x = 0;\ntry {\n    x = 1; // vynil-lint-disable rhai/empty-catch\n} catch {}\n";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/empty-catch"),
            "la directive entre le try et le catch doit supprimer rhai/empty-catch : {findings:?}"
        );
    }

    #[test]
    fn inline_disable_after_the_catch_does_not_suppress_empty_catch() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source =
            "let x = 0;\ntry {\n    x = 1;\n} catch {}\nx = 2; // vynil-lint-disable rhai/empty-catch\n";
        let findings = checker.check_file(&PathBuf::from("test.rhai"), source);

        assert!(
            findings.iter().any(|f| f.rule == "rhai/empty-catch"),
            "une directive après le bloc ne doit rien supprimer"
        );
    }

    #[test]
    fn k8s_resource_plural_kind_produces_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = r#"let s = k8s_resource("Secrets", "ns").get("my-secret");"#;
        let findings = checker.check_file(&PathBuf::from("install.rhai"), source);

        assert!(
            findings.iter().any(|f| f.rule == "rhai/k8s-resource-plural"),
            "Expected rhai/k8s-resource-plural warning for plural kind"
        );
    }

    #[test]
    fn k8s_resource_singular_kind_no_warning() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = r#"let s = k8s_resource("Secret", "ns").get("my-secret");"#;
        let findings = checker.check_file(&PathBuf::from("install.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/k8s-resource-plural"),
            "No warning expected for singular kind"
        );
    }

    #[test]
    fn context_namespace_access_in_system_package_warns() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = r"fn run(instance, context) {
    let replicas = if context.namespace.ha { 2 } else { 1 };
}";
        let findings = checker.check_file(&PathBuf::from("scripts/install.rhai"), source);

        assert!(
            findings.iter().any(|f| f.rule == "rhai/wrong-package-type"),
            "Expected rhai/wrong-package-type finding for context.namespace access in system package"
        );
    }

    #[test]
    fn context_tenant_access_in_system_package_warns() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = r"fn run(instance, context) {
    let name = context.tenant.name;
}";
        let findings = checker.check_file(&PathBuf::from("scripts/install.rhai"), source);

        assert!(
            findings.iter().any(|f| f.rule == "rhai/wrong-package-type"),
            "Expected rhai/wrong-package-type finding for context.tenant access in system package"
        );
    }

    #[test]
    fn context_cluster_access_in_system_package_no_warn() {
        let base_dir = get_fixture_dir("rhai-checks");
        let pkg = read_package_yaml(&base_dir.join("package.yaml")).expect("Failed to load package");
        let config = LintConfig::default();
        let mut checker = RhaiChecker::new(&base_dir, &base_dir, None, &pkg, &config);

        let source = r"fn run(instance, context) {
    let replicas = if context.cluster.ha { 2 } else { 1 };
}";
        let findings = checker.check_file(&PathBuf::from("scripts/install.rhai"), source);

        assert!(
            !findings.iter().any(|f| f.rule == "rhai/wrong-package-type"),
            "context.cluster is valid in system packages, should not warn"
        );
    }
}
