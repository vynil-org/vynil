//! Cobra `__complete` protocol for `kubectl` plugin shell completion.
//! Entry point: `kubectl-vynil __complete <words...>` (see docs/conception/completion.md).

use crate::cli::Cli;
use clap::CommandFactory;
use kube::{
    ResourceExt,
    api::{Api, DynamicObject, ListParams},
};
use std::time::Duration;
use tokio::time::timeout;

// ── Cobra directive bitfield ────────────────────────────────────────────────
const NO_FILE_COMP: u8 = 4;
const CLUSTER_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, PartialEq, Eq)]
pub struct Candidate {
    pub value: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KindId {
    Jukebox,
    Vti,
    Vsvc,
    Vsi,
}

impl KindId {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "jukebox" | "box" => Some(Self::Jukebox),
            "vti" | "tenantinstance" | "tenantinstances" => Some(Self::Vti),
            "vsvc" | "serviceinstance" | "serviceinstances" => Some(Self::Vsvc),
            "vsi" | "systeminstance" | "systeminstances" => Some(Self::Vsi),
            _ => None,
        }
    }

    /// Name of the corresponding `clap` subcommand in `enum Commands`.
    const fn subcommand_name(self) -> &'static str {
        match self {
            Self::Jukebox => "jukebox",
            Self::Vti => "vti",
            Self::Vsvc => "vsvc",
            Self::Vsi => "vsi",
        }
    }

    const fn kind_str(self) -> &'static str {
        match self {
            Self::Jukebox => "JukeBox",
            Self::Vti => crate::cli::TENANT_INSTANCE.kind,
            Self::Vsvc => crate::cli::SERVICE_INSTANCE.kind,
            Self::Vsi => crate::cli::SYSTEM_INSTANCE.kind,
        }
    }

    const fn plural(self) -> &'static str {
        match self {
            Self::Jukebox => "jukeboxes",
            Self::Vti => crate::cli::TENANT_INSTANCE.plural,
            Self::Vsvc => crate::cli::SERVICE_INSTANCE.plural,
            Self::Vsi => crate::cli::SYSTEM_INSTANCE.plural,
        }
    }

    const fn is_namespaced(self) -> bool {
        !matches!(self, Self::Jukebox)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Slot {
    Kind,
    Name(KindId),
    Verb(KindId),
    VerbFlag(KindId, String),
    GlobalFlagValue,
    None,
}

#[derive(Debug, PartialEq, Eq)]
struct Parsed {
    slot: Slot,
    to_complete: String,
    ns: Option<String>,
    context: Option<String>,
}

/// Splits the already-typed words into (slot, `to_complete`, ns, context).
fn classify(words: &[String]) -> Parsed {
    let Some((to_complete, typed)) = words.split_last() else {
        return Parsed {
            slot: Slot::Kind,
            to_complete: String::new(),
            ns: None,
            context: None,
        };
    };
    let to_complete = to_complete.clone();

    // Walk typed words, absorbing global flags and their values.
    let mut positionals: Vec<&str> = Vec::new();
    let mut ns: Option<String> = None;
    let mut context: Option<String> = None;
    let mut prev_bare_global = false;
    let mut words_iter = typed.iter();

    while let Some(word) = words_iter.next() {
        let w = word.as_str();
        prev_bare_global = false;

        if w == "-n" || w == "--namespace" {
            match words_iter.next() {
                Some(v) => ns = Some(v.clone()),
                None => prev_bare_global = true,
            }
            continue;
        }

        if w == "--context" {
            match words_iter.next() {
                Some(v) => context = Some(v.clone()),
                None => prev_bare_global = true,
            }
            continue;
        }

        if let Some(v) = w.strip_prefix("--namespace=") {
            ns = Some(v.to_string());
            continue;
        }

        if let Some(v) = w.strip_prefix("--context=") {
            context = Some(v.to_string());
            continue;
        }

        if w.starts_with('-') {
            // unknown flag: skip it, don't treat as positional
            continue;
        }

        positionals.push(w);
    }

    if prev_bare_global {
        return Parsed {
            slot: Slot::GlobalFlagValue,
            to_complete,
            ns,
            context,
        };
    }

    // Completing a flag?
    if to_complete.starts_with('-') {
        let slot = match (
            positionals.first().and_then(|&p| KindId::parse(p)),
            positionals.get(2),
        ) {
            (Some(k), Some(verb)) if positionals.len() > 2 => Slot::VerbFlag(k, verb.to_string()),
            _ => Slot::Kind,
        };
        return Parsed {
            slot,
            to_complete,
            ns,
            context,
        };
    }

    let slot = match positionals.len() {
        0 => Slot::Kind,
        1 => positionals
            .first()
            .and_then(|&p| KindId::parse(p))
            .map_or(Slot::None, Slot::Name),
        2 => positionals
            .first()
            .and_then(|&p| KindId::parse(p))
            .map_or(Slot::None, Slot::Verb),
        _ => Slot::None,
    };

    Parsed {
        slot,
        to_complete,
        ns,
        context,
    }
}

/// First line of a clap help/about string, tabs stripped.
fn one_line(s: Option<&clap::builder::StyledStr>) -> Option<String> {
    let raw = s?.to_string();
    let first = raw
        .lines()
        .next()
        .unwrap_or("")
        .replace('\t', " ")
        .trim()
        .to_string();
    (!first.is_empty()).then_some(first)
}

fn kind_candidates() -> Vec<Candidate> {
    let cmd = Cli::command();
    let mut out = Vec::new();
    for sub in cmd.get_subcommands() {
        if sub.is_hide_set() {
            continue;
        }
        let about = one_line(sub.get_about());
        out.push(Candidate {
            value: sub.get_name().to_string(),
            description: about.clone(),
        });
        for alias in sub.get_visible_aliases() {
            out.push(Candidate {
                value: alias.to_string(),
                description: about.clone(),
            });
        }
    }
    out
}

fn verb_candidates(kind: KindId) -> Vec<Candidate> {
    let cmd = Cli::command();
    let Some(sub) = cmd
        .get_subcommands()
        .find(|s| s.get_name() == kind.subcommand_name())
    else {
        return vec![];
    };
    sub.get_subcommands()
        .filter(|v| !v.is_hide_set())
        .map(|v| Candidate {
            value: v.get_name().to_string(),
            description: one_line(v.get_about()),
        })
        .collect()
}

fn global_flag_candidates() -> Vec<Candidate> {
    Cli::command()
        .get_arguments()
        .filter(|a| a.is_global_set())
        .filter_map(|a| {
            a.get_long().map(|l| Candidate {
                value: format!("--{l}"),
                description: one_line(a.get_help()),
            })
        })
        .collect()
}

fn verb_flag_candidates(kind: KindId, verb: &str) -> Vec<Candidate> {
    let cmd = Cli::command();
    let Some(sub) = cmd
        .get_subcommands()
        .find(|s| s.get_name() == kind.subcommand_name())
    else {
        return vec![];
    };
    let Some(v) = sub.get_subcommands().find(|v| v.get_name() == verb) else {
        return vec![];
    };
    v.get_arguments()
        .filter_map(|a| {
            a.get_long().map(|l| Candidate {
                value: format!("--{l}"),
                description: one_line(a.get_help()),
            })
        })
        .collect()
}

/// Read the namespace of a kubeconfig context. Best-effort: returns "default" on error.
async fn namespace_of_context(context: Option<&str>) -> String {
    use kube::config::{Config, KubeConfigOptions};

    let config = if let Some(ctx) = context {
        let options = KubeConfigOptions {
            context: Some(ctx.to_string()),
            cluster: None,
            user: None,
        };
        Config::from_kubeconfig(&options).await.ok()
    } else {
        Config::infer().await.ok()
    };

    config.map_or_else(|| "default".to_string(), |cfg| cfg.default_namespace)
}

/// List resource names from a cluster for the given kind and namespace.
/// Returns empty vec on timeout, error, or missing client.
async fn list_names(kind: KindId, ns: Option<&str>, context: Option<&str>) -> Vec<Candidate> {
    let Ok(Ok(client)) = timeout(CLUSTER_TIMEOUT, crate::transport::make_client(context)).await else {
        return vec![];
    };

    // Determine the namespace to use (explicit arg or inferred from context)
    let ns_string = if let Some(n) = ns {
        n.to_string()
    } else {
        namespace_of_context(context).await
    };

    let ar = crate::actions::vynil_api_resource(kind.kind_str(), kind.plural());
    let api: Api<DynamicObject> = if kind.is_namespaced() {
        Api::namespaced_with(client, &ns_string, &ar)
    } else {
        Api::all_with(client, &ar)
    };

    let lp = ListParams::default().limit(500);
    match timeout(CLUSTER_TIMEOUT, api.list(&lp)).await {
        Ok(Ok(list)) => list
            .items
            .iter()
            .map(|o| Candidate {
                value: o.name_any(),
                description: None,
            })
            .collect(),
        _ => vec![],
    }
}

/// Render candidates and directive to Cobra format (tab-separated, :<directive> on last line).
fn render(candidates: &[Candidate], to_complete: &str) -> String {
    let mut out = String::new();
    for c in candidates.iter().filter(|c| c.value.starts_with(to_complete)) {
        out.push_str(&c.value);
        if let Some(d) = &c.description
            && !d.is_empty()
        {
            out.push('\t');
            out.push_str(d);
        }
        out.push('\n');
    }
    out.push(':');
    let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{NO_FILE_COMP}\n"));
    out
}

fn emit(candidates: &[Candidate], to_complete: &str) {
    print!("{}", render(candidates, to_complete));
}

pub async fn run(words: Vec<String>) {
    let parsed = classify(&words);
    let candidates = match parsed.slot {
        Slot::Kind => {
            if parsed.to_complete.starts_with('-') {
                global_flag_candidates()
            } else {
                kind_candidates()
            }
        }
        Slot::Verb(k) => verb_candidates(k),
        Slot::VerbFlag(k, ref v) => verb_flag_candidates(k, v),
        Slot::Name(k) => list_names(k, parsed.ns.as_deref(), parsed.context.as_deref()).await,
        Slot::GlobalFlagValue | Slot::None => vec![],
    };
    emit(&candidates, &parsed.to_complete);
}

pub fn emit_script(shell: crate::cli::CompletionShell) {
    let s = match shell {
        crate::cli::CompletionShell::Bash => include_str!("../completions/bash.sh"),
        crate::cli::CompletionShell::Zsh => include_str!("../completions/zsh.sh"),
    };
    print!("{s}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(strs: &[&str]) -> Vec<String> {
        strs.iter().map(std::string::ToString::to_string).collect()
    }

    // ── Automate tests (classify) ──────────────────────────────────────────

    #[test]
    fn classify_empty_completion() {
        let parsed = classify(&args(&[""]));
        assert_eq!(parsed.slot, Slot::Kind);
        assert_eq!(parsed.to_complete, "");
        assert_eq!(parsed.ns, None);
        assert_eq!(parsed.context, None);
    }

    #[test]
    fn classify_partial_kind() {
        let parsed = classify(&args(&["v"]));
        assert_eq!(parsed.slot, Slot::Kind);
        assert_eq!(parsed.to_complete, "v");
        assert_eq!(parsed.ns, None);
        assert_eq!(parsed.context, None);
    }

    #[test]
    fn classify_kind_then_name() {
        let parsed = classify(&args(&["vti", ""]));
        assert_eq!(parsed.slot, Slot::Name(KindId::Vti));
        assert_eq!(parsed.to_complete, "");
        assert_eq!(parsed.ns, None);
        assert_eq!(parsed.context, None);
    }

    #[test]
    fn classify_kind_alias_then_name() {
        let parsed = classify(&args(&["tenantinstance", ""]));
        assert_eq!(parsed.slot, Slot::Name(KindId::Vti));
        assert_eq!(parsed.to_complete, "");
        assert_eq!(parsed.ns, None);
        assert_eq!(parsed.context, None);
    }

    #[test]
    fn classify_kind_name_then_verb() {
        let parsed = classify(&args(&["vti", "x", ""]));
        assert_eq!(parsed.slot, Slot::Verb(KindId::Vti));
        assert_eq!(parsed.to_complete, "");
        assert_eq!(parsed.ns, None);
        assert_eq!(parsed.context, None);
    }

    #[test]
    fn classify_kind_name_partial_verb() {
        let parsed = classify(&args(&["vti", "x", "up"]));
        assert_eq!(parsed.slot, Slot::Verb(KindId::Vti));
        assert_eq!(parsed.to_complete, "up");
        assert_eq!(parsed.ns, None);
        assert_eq!(parsed.context, None);
    }

    #[test]
    fn classify_verb_with_flag() {
        let parsed = classify(&args(&["vti", "x", "upgrade", "--"]));
        assert_eq!(parsed.slot, Slot::VerbFlag(KindId::Vti, "upgrade".to_string()));
        assert_eq!(parsed.to_complete, "--");
        assert_eq!(parsed.ns, None);
        assert_eq!(parsed.context, None);
    }

    #[test]
    fn classify_global_flag_before_kind() {
        let parsed = classify(&args(&["-n", "prod", "vti", ""]));
        assert_eq!(parsed.slot, Slot::Name(KindId::Vti));
        assert_eq!(parsed.to_complete, "");
        assert_eq!(parsed.ns, Some("prod".to_string()));
        assert_eq!(parsed.context, None);
    }

    #[test]
    fn classify_verb_with_global_flag_bare() {
        let parsed = classify(&args(&["vti", "x", "upgrade", "-n", ""]));
        assert_eq!(parsed.slot, Slot::GlobalFlagValue);
        assert_eq!(parsed.to_complete, "");
        assert_eq!(parsed.ns, None);
        assert_eq!(parsed.context, None);
    }

    #[test]
    fn classify_jukebox_then_name() {
        let parsed = classify(&args(&["box", ""]));
        assert_eq!(parsed.slot, Slot::Name(KindId::Jukebox));
        assert_eq!(parsed.to_complete, "");
        assert_eq!(parsed.ns, None);
        assert_eq!(parsed.context, None);
    }

    #[test]
    fn classify_unknown_kind() {
        let parsed = classify(&args(&["nope", ""]));
        assert_eq!(parsed.slot, Slot::None);
        assert_eq!(parsed.to_complete, "");
        assert_eq!(parsed.ns, None);
        assert_eq!(parsed.context, None);
    }

    #[test]
    fn classify_global_flag_bare() {
        let parsed = classify(&args(&["-n", ""]));
        assert_eq!(parsed.slot, Slot::GlobalFlagValue);
        assert_eq!(parsed.to_complete, "");
        assert_eq!(parsed.ns, None);
        assert_eq!(parsed.context, None);
    }

    // ── New tests for ns/context extraction ──────────────────────────────────

    #[test]
    fn classify_namespace_short_flag() {
        let parsed = classify(&args(&["-n", "prod", "vti", ""]));
        assert_eq!(parsed.ns, Some("prod".to_string()));
        assert_eq!(parsed.slot, Slot::Name(KindId::Vti));
    }

    #[test]
    fn classify_namespace_long_flag() {
        let parsed = classify(&args(&["--namespace", "dev", "vti", ""]));
        assert_eq!(parsed.ns, Some("dev".to_string()));
        assert_eq!(parsed.slot, Slot::Name(KindId::Vti));
    }

    #[test]
    fn classify_namespace_long_flag_equals() {
        let parsed = classify(&args(&["--namespace=dev", "vti", ""]));
        assert_eq!(parsed.ns, Some("dev".to_string()));
        assert_eq!(parsed.slot, Slot::Name(KindId::Vti));
    }

    #[test]
    fn classify_context_long_flag() {
        let parsed = classify(&args(&["--context", "stg", "box", ""]));
        assert_eq!(parsed.context, Some("stg".to_string()));
        assert_eq!(parsed.slot, Slot::Name(KindId::Jukebox));
    }

    #[test]
    fn classify_context_long_flag_equals() {
        let parsed = classify(&args(&["--context=stg", "box", ""]));
        assert_eq!(parsed.context, Some("stg".to_string()));
        assert_eq!(parsed.slot, Slot::Name(KindId::Jukebox));
    }

    // ── Static candidates tests ────────────────────────────────────────────

    #[test]
    fn kind_candidates_contains_jukebox() {
        let candidates = kind_candidates();
        assert!(candidates.iter().any(|c| c.value == "jukebox"));
    }

    #[test]
    fn kind_candidates_contains_vti() {
        let candidates = kind_candidates();
        assert!(candidates.iter().any(|c| c.value == "vti"));
    }

    #[test]
    fn kind_candidates_contains_vsvc() {
        let candidates = kind_candidates();
        assert!(candidates.iter().any(|c| c.value == "vsvc"));
    }

    #[test]
    fn kind_candidates_contains_vsi() {
        let candidates = kind_candidates();
        assert!(candidates.iter().any(|c| c.value == "vsi"));
    }

    #[test]
    fn kind_candidates_excludes_hidden_complete() {
        let candidates = kind_candidates();
        assert!(!candidates.iter().any(|c| c.value == "__complete"));
    }

    #[test]
    fn verb_candidates_for_vti() {
        let candidates = verb_candidates(KindId::Vti);
        let values: Vec<&str> = candidates.iter().map(|c| c.value.as_str()).collect();
        let expected = vec![
            "upgrade",
            "scan",
            "diagnostic",
            "children",
            "agentlog",
            "childlogs",
            "operatorlog",
        ];
        for verb in &expected {
            assert!(values.contains(verb), "verb {verb} should be in VTI candidates");
        }
        assert_eq!(
            values.len(),
            expected.len(),
            "should have exactly {} verbs",
            expected.len()
        );
    }

    #[test]
    fn verb_candidates_for_jukebox() {
        let candidates = verb_candidates(KindId::Jukebox);
        let values: Vec<&str> = candidates.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values, vec!["scan"]);
    }

    #[test]
    fn render_with_two_candidates_one_described() {
        let candidates = vec![
            Candidate {
                value: "upgrade".to_string(),
                description: Some("Force-reinstall".to_string()),
            },
            Candidate {
                value: "scan".to_string(),
                description: None,
            },
        ];
        let output = render(&candidates, "");
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3); // upgrade + description, scan, :4
        assert_eq!(lines[0], "upgrade\tForce-reinstall");
        assert_eq!(lines[1], "scan");
        assert_eq!(lines[2], ":4");
    }

    #[test]
    fn render_empty_candidates() {
        let candidates: Vec<Candidate> = vec![];
        let output = render(&candidates, "");
        assert_eq!(output, ":4\n");
    }

    #[test]
    fn render_filters_by_to_complete() {
        let candidates = vec![
            Candidate {
                value: "upgrade".to_string(),
                description: None,
            },
            Candidate {
                value: "scan".to_string(),
                description: None,
            },
        ];
        let output = render(&candidates, "up");
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 2); // upgrade, :4
        assert_eq!(lines[0], "upgrade");
        assert_eq!(lines[1], ":4");
    }

    // ── list_names tests (cluster fallback) ──────────────────────────────────

    #[tokio::test]
    async fn list_names_bad_context_returns_empty() {
        // Test with an invalid context: should timeout and return empty vec.
        let result = list_names(KindId::Vti, None, Some("nonexistent-context-xyz")).await;
        assert_eq!(result, vec![]);
    }

    #[test]
    fn emit_script_bash_is_not_empty() {
        let bash_script = include_str!("../completions/bash.sh");
        assert_ne!(bash_script, "");
        assert!(bash_script.contains("__complete"));
    }

    #[test]
    fn emit_script_zsh_is_not_empty() {
        let zsh_script = include_str!("../completions/zsh.sh");
        assert_ne!(zsh_script, "");
        assert!(zsh_script.contains("__complete"));
    }
}
