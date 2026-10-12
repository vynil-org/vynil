//! Fonction `regex_is_match` offerte au script de lint (spec : `patterns.sdd`).

use regex::RegexBuilder;
use rhai::{Dynamic, Engine, EvalAltResult, Position};

/// Taille maximale du programme compilé d'un motif (octets). Au-delà, le motif est refusé
/// plutôt que de consommer la mémoire du processus.
const COMPILED_SIZE_LIMIT: usize = 1 << 20;

/// Enregistre `regex_is_match(pattern, text) -> bool` dans `engine`.
pub fn register(engine: &mut Engine) {
    engine.register_fn("regex_is_match", regex_is_match);
}

/// Vrai quand `pattern` trouve une correspondance dans `text`. Motif invalide ou trop grand :
/// erreur d'exécution Rhai, jamais un panic.
fn regex_is_match(pattern: &str, text: &str) -> Result<bool, Box<EvalAltResult>> {
    let regex = RegexBuilder::new(pattern)
        .size_limit(COMPILED_SIZE_LIMIT)
        .build()
        .map_err(|e| {
            Box::new(EvalAltResult::ErrorRuntime(
                Dynamic::from(format!("regex_is_match: invalid pattern {pattern}: {e}")),
                Position::NONE,
            ))
        })?;
    Ok(regex.is_match(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> Engine {
        let mut engine = Engine::new();
        register(&mut engine);
        engine
    }

    fn eval(script: &str) -> Result<bool, Box<rhai::EvalAltResult>> {
        engine().eval::<bool>(script)
    }

    #[test]
    fn an_unanchored_pattern_matches_inside_the_text() {
        let found = eval(r#"regex_is_match("latest", "registry/app:latest")"#);
        assert!(found.expect("évaluation"));
    }

    #[test]
    fn anchors_constrain_the_match() {
        let on_app = eval(r#"regex_is_match("^app$", "app")"#);
        let on_path = eval(r#"regex_is_match("^app$", "registry/app")"#);
        assert!(on_app.expect("évaluation"));
        assert!(!on_path.expect("évaluation"));
    }

    #[test]
    fn the_default_latest_tag_block_list() {
        let script = r#"
            fn hit(text) {
                let block = [".*:(latest)$", "^[^:]*$", "(.*/[^:]+)$"];
                for p in block { if regex_is_match(p, text) { return true; } }
                false
            }
            [hit("nginx:latest"), hit("nginx"), hit("nginx:1.27")]
        "#;
        let hits = engine().eval::<rhai::Array>(script).expect("évaluation");
        let flags: Vec<bool> = hits.into_iter().map(|d| d.as_bool().expect("booléen")).collect();
        assert_eq!(flags, vec![true, true, false]);
    }

    #[test]
    fn an_invalid_pattern_is_a_script_error() {
        let err = eval(r#"regex_is_match("(", "x")"#).expect_err("erreur attendue");
        let rhai::EvalAltResult::ErrorRuntime(payload, _) = *err else {
            panic!("erreur d'exécution attendue");
        };
        let message = payload.to_string();
        assert!(
            message.starts_with("regex_is_match: invalid pattern"),
            "message inattendu : {message}"
        );
    }

    #[test]
    fn a_pathological_pattern_stays_linear() {
        let text = format!("{}b", "a".repeat(50));
        let script = format!(r#"regex_is_match("(a+)+$", "{text}")"#);
        assert!(!eval(&script).expect("évaluation"));
    }
}
