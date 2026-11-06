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

### Scripts Rhai (`rhai/`)

- validité de syntaxe ;
- imports non résolus ;
- code mort, variables inutilisées, variables masquées (shadowing) ;
- paramètres et fonctions inutilisés (`rhai/unused-function`, `rhai/unused-variable`) ;
- validation du mode d'API (pas de full-API dans les scripts core) ;
- validation du type de paquet (pas d'accès tenant dans un paquet système) ;
- validation du retour des hooks de contexte (`rhai/context-hook-no-return`).

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
