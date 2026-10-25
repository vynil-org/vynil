//! Gabarits Handlebars offerts au script de lint, bornés au répertoire de la distribution
//! (spec : `templates.sdd`).

use common::{RhaiRes, handlebarshandler::HandleBars};
use rhai::{Dynamic, Engine, EvalAltResult, Position};
use std::path::{Component, Path, PathBuf};

/// Enregistre le type `HandleBars` et ses cinq fonctions dans `engine`.
///
/// Les moteurs créés par `new_hbs` sont restreints ; `register_partial_dir` ne lit que sous
/// `lint_dir`, et `register_helper_dir` n'est pas enregistré (aucun chargement de code).
pub fn register(engine: &mut Engine, lint_dir: &Path) {
    let lint_dir = lint_dir.to_path_buf();
    engine
        .register_type_with_name::<HandleBars<'_>>("HandleBars")
        .register_fn("new_hbs", HandleBars::new_restricted)
        .register_fn(
            "register_template",
            |h: &mut HandleBars, name: String, template: String| -> RhaiRes<()> {
                h.rhai_register_template(name, template)
            },
        )
        .register_fn(
            "register_partial_dir",
            move |h: &mut HandleBars, dir: &str| -> RhaiRes<()> {
                let resolved = partial_dir(&lint_dir, dir)?;
                h.rhai_register_partial_dir(path_string(&resolved, dir)?)
            },
        )
        .register_fn(
            "render_from",
            |h: &mut HandleBars, template: String, data: rhai::Map| -> RhaiRes<String> {
                h.rhai_render(template, data)
            },
        )
        .register_fn(
            "render_named",
            |h: &mut HandleBars, name: String, template: String, data: rhai::Map| -> RhaiRes<String> {
                h.rhai_render_named(name, template, data)
            },
        );
}

/// Erreur de script, jamais un panic, qui nomme `dir` et dit pourquoi on la refuse.
fn refusal<T>(dir: &str, reason: &str) -> RhaiRes<T> {
    Err(Box::new(EvalAltResult::ErrorRuntime(
        Dynamic::from(format!("register_partial_dir({dir}) : {reason}")),
        Position::NONE,
    )))
}

/// Résout `dir` (relatif à `lint_dir`, liens symboliques suivis) en un répertoire situé dans
/// `lint_dir`. Toute autre situation est une erreur de script qui nomme `dir`.
fn partial_dir(lint_dir: &Path, dir: &str) -> RhaiRes<PathBuf> {
    let requested = Path::new(dir);
    if requested.is_absolute() {
        return refusal(
            dir,
            "chemin absolu refusé, donner un chemin relatif à la distribution",
        );
    }
    if requested.components().any(|c| c == Component::ParentDir) {
        return refusal(dir, "composant `..` refusé");
    }
    let root = lint_dir
        .canonicalize()
        .or_else(|e| refusal(dir, &format!("répertoire de lint illisible : {e}")))?;
    let resolved = lint_dir
        .join(requested)
        .canonicalize()
        .or_else(|_| refusal(dir, "répertoire introuvable"))?;
    if !resolved.is_dir() {
        return refusal(dir, "n'est pas un répertoire");
    }
    if !resolved.starts_with(&root) {
        return refusal(dir, "sort du répertoire de lint");
    }
    if let Some(escaping) = escaping_entry(&resolved, &root) {
        return refusal(
            dir,
            &format!("`{}` sort du répertoire de lint", escaping.display()),
        );
    }
    Ok(resolved)
}

/// Première entrée de `dir` (récursivement) qui est un lien symbolique de fichier dont la cible
/// canonique sort de `root` : un `.hbs` lié vers l'extérieur serait sinon lu. Les liens de
/// répertoire ne sont pas suivis par le parcours de `handlebars`, donc pas examinés.
fn escaping_entry(dir: &Path, root: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_symlink() {
            match path.canonicalize() {
                Ok(target) if target.is_dir() => {}
                Ok(target) if target.starts_with(root) => {}
                _ => return Some(path),
            }
        } else if kind.is_dir()
            && let Some(found) = escaping_entry(&path, root)
        {
            return Some(found);
        }
    }
    None
}

/// Chemin résolu sous forme de chaîne, pour l'API `rhai_register_partial_dir`.
fn path_string(path: &Path, requested: &str) -> RhaiRes<String> {
    path.to_str().map_or_else(
        || refusal(requested, "chemin non UTF-8"),
        |text| Ok(text.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhai::packages::Package;
    use std::fs;

    /// Moteur comme `build_engine` le compose (paquets de données de base), plus `register`.
    fn engine(lint_dir: &Path) -> Engine {
        let mut engine = Engine::new_raw();
        engine.register_global_module(rhai::packages::CorePackage::new().as_shared_module());
        engine.register_global_module(rhai::packages::LogicPackage::new().as_shared_module());
        engine.register_global_module(rhai::packages::BasicMapPackage::new().as_shared_module());
        engine.register_global_module(rhai::packages::BasicStringPackage::new().as_shared_module());
        engine.register_global_module(rhai::packages::BasicArrayPackage::new().as_shared_module());
        register(&mut engine, lint_dir);
        engine
    }

    #[test]
    fn a_script_renders_a_template_from_a_string() {
        let dir = tempfile::tempdir().expect("répertoire temporaire");
        let engine = engine(dir.path());
        let out: String = engine
            .eval(r#"let h = new_hbs(); render_from(h, "Bonjour {{name}}", #{ name: "Zoé" })"#)
            .expect("rendu");
        assert_eq!(out, "Bonjour Zoé");

        let out: String = engine
            .eval(
                r#"
                let h = new_hbs();
                register_template(h, "salut", "Salut {{name}}");
                render_named(h, "accueil", "{{> salut}} !", #{ name: "Zoé" })
                "#,
            )
            .expect("rendu nommé");
        assert_eq!(out, "Salut Zoé !");
    }

    #[test]
    fn partials_come_from_the_lint_directory() {
        let dir = tempfile::tempdir().expect("répertoire temporaire");
        let templates = dir.path().join("templates");
        fs::create_dir_all(&templates).expect("templates/");
        fs::write(templates.join("hint.hbs"), "{{> intro}} : {{rule}}").expect("hint.hbs");
        fs::write(templates.join("intro.hbs"), "Hint").expect("intro.hbs");
        fs::write(templates.join("notes.txt"), "pas un gabarit").expect("notes.txt");
        let engine = engine(dir.path());

        let out: String = engine
            .eval(
                r#"
                let h = new_hbs();
                register_partial_dir(h, "templates");
                render_from(h, "{{> hint}}", #{ rule: "no-cpu-limits" })
                "#,
            )
            .expect("rendu");
        assert_eq!(out, "Hint : no-cpu-limits");

        let notes = engine.eval::<String>(
            r#"let h = new_hbs(); register_partial_dir(h, "templates"); render_from(h, "{{> notes}}", #{})"#,
        );
        assert!(notes.is_err(), "notes.txt ne doit rien enregistrer : {notes:?}");
    }

    #[test]
    fn the_directory_is_bounded_to_the_lint_directory() {
        let root = tempfile::tempdir().expect("répertoire temporaire");
        let lint = root.path().join("distro");
        let sibling = root.path().join("frère");
        fs::create_dir_all(&lint).expect("lint dir");
        fs::create_dir_all(&sibling).expect("frère");
        fs::write(sibling.join("secret.hbs"), "SECRET").expect("secret.hbs");
        std::os::unix::fs::symlink(&sibling, lint.join("lien")).expect("lien symbolique");
        let engine = engine(&lint);

        let absolute = sibling.to_str().expect("chemin UTF-8").to_string();
        for dir in ["..", absolute.as_str(), "../frère", "lien", "absent"] {
            let script = format!(r#"let h = new_hbs(); register_partial_dir(h, "{dir}");"#);
            let err = engine.eval::<()>(&script).expect_err(dir);
            assert!(
                err.to_string().contains(dir),
                "le message doit nommer {dir} : {err}"
            );
        }

        engine
            .eval::<()>(r#"let h = new_hbs(); register_partial_dir(h, ".");"#)
            .expect("le répertoire de lint lui-même est accepté");
        let leaked = engine.eval::<String>(r#"let h = new_hbs(); render_from(h, "{{> secret}}", #{})"#);
        assert!(
            leaked.is_err(),
            "secret.hbs hors du répertoire de lint ne doit pas être lu"
        );
    }

    #[test]
    // Liste d'exclusion : tous les helpers de `handlebars_misc_helpers` 0.17 qui touchent à l'hôte
    // (fichier, environnement, réseau) y figurent. Un bump de `vynil-core` ou de la crate doit
    // relire ses `register_helper` et compléter cette liste.
    fn host_helpers_are_gone() {
        let dir = tempfile::tempdir().expect("répertoire temporaire");
        let engine = engine(dir.path());
        for template in [
            r#"{{render_file "x" this}}"#,
            r#"{{read_to_str "x"}}"#,
            r#"{{env_var "PATH"}}"#,
            r#"{{canonicalize "."}}"#,
            r#"{{http_get "http://127.0.0.1/"}}"#,
            r#"{{gitignore_io "rust"}}"#,
            r#"{{#jsonnet}}importstr "/etc/hostname"{{/jsonnet}}"#,
        ] {
            let mut scope = rhai::Scope::new();
            scope.push("tpl", template.to_string());
            let out = engine.eval_with_scope::<String>(&mut scope, "render_from(new_hbs(), tpl, #{})");
            assert!(out.is_err(), "{template} doit échouer, obtenu {out:?}");
        }

        let path = std::env::var("PATH").expect("PATH de test");
        let mut scope = rhai::Scope::new();
        scope.push(
            "tpl",
            r#"{{render_template "{{env_var \"PATH\"}}" this}}"#.to_string(),
        );
        let nested: String = engine
            .eval_with_scope(&mut scope, "render_from(new_hbs(), tpl, #{})")
            .expect("le helper imbriqué avale son échec");
        assert!(!nested.contains(&path), "le PATH de l'hôte est rendu : {nested}");

        let out: String = engine
            .eval(r#"render_from(new_hbs(), "{{concat \"a\" \"b\"}}{{base64_encode \"a\"}}", #{})"#)
            .expect("les helpers purs rendent");
        assert_eq!(out, "abYQ==");
    }

    #[test]
    fn no_way_to_load_code() {
        let dir = tempfile::tempdir().expect("répertoire temporaire");
        let engine = engine(dir.path());
        let err = engine
            .eval::<String>(r#"let h = new_hbs(); register_helper_dir(h, "templates"); "ok""#)
            .expect_err("register_helper_dir doit être inconnu");
        assert!(err.to_string().contains("not found"), "{err}");
    }
}
