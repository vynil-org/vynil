# Lint d'un paquet

`agent package lint` analyse un paquet **statiquement**, sans déploiement : structure,
templates Handlebars et scripts Rhai. C'est l'outil à brancher en CI avant publication.

```bash
agent package lint -p ./my-package --format junit --junit-output-filename lint.xml
```

| Option | Défaut | Rôle |
|---|---|---|
| `-p`, `--package-dir` | `/tmp/package` | Répertoire du paquet. |
| `-c`, `--config-dir` | `.` | Scripts Rhai additionnels. |
| `--format` | `text` | `text` ou `junit`. |
| `--level` | `all` | Sévérité minimale affichée : `error`, `warn`, `all`. |
| `--junit-output-filename` | — | Écrit un rapport JUnit XML. |

## Vérifications effectuées

### Structure (`package/`)

- présence et format de `package.yaml` ;
- structure de répertoires requise ;
- définitions de ressources.

### Templates Handlebars (`hbs/`)

- validité de syntaxe ;
- helpers inconnus (`hbs/unknown-helper`) ;
- partials inconnus ;
- cohérence des variables de contexte avec `package.yaml` ;
- validité des clés de ressources et d'images ;
- usage du bon type de paquet.

Pour `hbs/unused-option`, l'usage est compté depuis les templates et depuis les scripts
Rhai, directement (`context.values.…`) ou par alias simple de `context.values`
(`let v = context.values ; v.ingress.provider`).

L'analyse des alias est textuelle, sans portée lexicale : deux variables de même nom dans des
fonctions différentes fusionnent leurs chemins, et une chaîne ou un commentaire contenant
`let x = context.values;` compte comme alias ; une option peut donc être jugée utilisée à tort
(un avertissement manqué, jamais un avertissement en trop). Un accès par indexation (`v["a"]`)
n'est pas suivi et produit toujours `hbs/unused-option`.

### Scripts Rhai (`rhai/`)

- validité de syntaxe ;
- imports non résolus ;
- code mort, variables inutilisées, variables masquées (shadowing) ;
- paramètres et fonctions inutilisés (`rhai/unused-function`, `rhai/unused-variable`) ;
- validation du mode d'API (pas de full-API dans les scripts core) ;
- validation du type de paquet (pas d'accès tenant dans un paquet système) ;
- validation du retour des hooks de contexte (`rhai/context-hook-no-return`) ;
- noms d'helpers appelés sans module alors qu'une forme qualifiée existe (`rhai/prefer-module`, avertissement) : `log_info(...)` devient `host::log_info(...)`, `json_encode(...)` devient `core::json_encode(...)`, `semver_from(...)` devient `semver::from(...)`, idem pour les modules `crypto` et `password`. Les appels `http`, `k8s`, `s3` et `oci` ne sont pas encore signalés.

## Configuration : `.vynil-lint.yaml`

Placé à la racine du paquet, il personnalise le comportement.

```yaml
disable:
  - rhai/unused-variable
  - hbs/unused-helper

override:
  rhai/unused-function: error
  hbs/unknown-helper: warn

files:
  - glob: "handlebars/helpers/**"
    disable:
      - rhai/unused-function
  - glob: "scripts/context_*.rhai"
    override:
      rhai/context-hook-no-return: error
```

## Désactivation inline

Désactiver une règle sur une seule ligne :

- Rhai : `// vynil-lint-disable rhai/unused-variable`
- Handlebars : `{{!-- vynil-lint-disable hbs/unknown-helper --}}`

Support du mode block (enable/disable)

## Codes de sortie

| Code | Signification |
|---|---|
| `0` | Aucun problème. |
| `1` | Erreurs détectées. |
| `2` | Uniquement des warnings. |
