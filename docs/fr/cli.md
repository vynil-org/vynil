# Référence CLI de l'agent

L'agent (`agent`) est le binaire exécuté dans les Jobs Kubernetes, mais il s'utilise aussi
en ligne de commande pour développer et tester les paquets. Les sous-commandes sont
définies dans [`agent/src/main.rs`](../../agent/src/main.rs).

```text
agent <COMMAND>
  package   gestion du cycle de vie des paquets (build/lint/update/test/validate/unpack)
  system    opérations sur une SystemInstance (install/delete/…)
  service   opérations sur une ServiceInstance
  tenant    opérations sur une TenantInstance
  box       opérations sur une JukeBox (scan, file-scan)
  template  rendu de templates
  run       exécute un dépôt git comme source de JukeBox
  crdgen    génère les manifestes CRD
  version   affiche la version
```

## `agent package`

| Sous-commande | Rôle |
|---|---|
| `build` | Packe un répertoire en image OCI et la pousse (option `--signing-key` pour signer, voir [Build & signature](build-signing.md)). |
| `unpack` | Télécharge et extrait une image de paquet dans un répertoire. |
| `update` | Met à jour les tags d'images dans `package.yaml` en interrogeant les registres, puis lance le hook `update_post` (régénération des templates). |
| `lint` | Analyse statique du paquet (structure, Handlebars, Rhai). Voir [Lint](tooling/lint.md). |
| `test` | Exécute les tests du paquet avec mocks K8s/HTTP. Voir [Tests de paquet](tooling/test.md). |
| `validate` | Valide le `package.yaml` (schéma, options). |

### `agent package lint`

```text
agent package lint -p <package-dir> [-c <config-dir>]
                   [--format text|junit] [--level error|warn|all]
                   [--junit-output-filename <file>]
```

### `agent package test`

```text
agent package test  -p <package-dir>
                   [--test-name <name> | --all]
                   [--testsets-dir <dir>]
                   [--format text|json]
                   [--junit-output-filename <file>]
                   [--template-output-filename <file>]   # pour un test uniquement => usage yamllint, kubelinter, ....
                   [--lint-dir <dir> [--schemas-dir <dir>]]
```

Le répertoire `<package-dir>/tests` doit exister (sinon erreur `MissingTestDirectory`).

| Flag | Variable d'environnement | Rôle |
|---|---|---|
| `--lint-dir <dir>` | `LINT_DIRECTORY` | Répertoire du script de lint du rendu de la distribution (`lint.rhai`). Quand il est donné, le script s'exécute à la fin de chaque test et ses constats s'ajoutent au verdict. Sans lui, aucun lint du rendu ne tourne. |
| `--schemas-dir <dir>` | `SCHEMAS_DIRECTORY` | Répertoire de JSON Schemas et de CRD utilisé par les fonctions `schema_*` du script de lint. Exige `--lint-dir` (erreur d'usage sinon) : `SCHEMAS_DIRECTORY` seule dans l'environnement est cette même erreur d'usage. |

Voir [Tests de paquet](tooling/test.md#lint-du-rendu) et [Lint du rendu](tooling/render-lint.md).

## `agent box`

| Sous-commande | Rôle |
|---|---|
| `scan` | Scanne une JukeBox (utilisé par le CronJob de l'opérateur). |
| `file-scan` | Scan standalone vers fichiers (`index.yaml` + fichiers de paquets), sans connexion Kubernetes. Voir [Réconciliation](reconciliation.md#scan-standalone-box-file-scan). |

## `agent {system,service,tenant}`

Opérations d'instance exécutées dans les Jobs : `install`, `delete`, `reconfigure`, et —
pour service/tenant — `backup`, `restore`. Paramètres communs (via flags ou variables
d'environnement) :

| Flag | Env | Défaut | Rôle |
|---|---|---|---|
| `-n`, `--namespace` | `NAMESPACE` | — | Namespace de l'instance. |
| `-i`, `--instance` | `INSTANCE` | — | Nom de l'instance. |
| `-v`, `--vynil-namespace` | `VYNIL_NAMESPACE` | — | Namespace système de Vynil. |
| `-p`, `--package-dir` | `PACKAGE_DIRECTORY` | `/tmp/package` | Répertoire du paquet dépaqueté. |
| `-s`, `--script-dir` | `SCRIPT_DIRECTORY` | `./agent/scripts` | Scripts d'agent. |
| `-t`, `--template-dir` | `TEMPLATE_DIRECTORY` | `./agent/templates` | Templates d'agent. |
| `-c`, `--config-dir` | `CONFIG_DIR` | `.` | Scripts Rhai additionnels. |
| `--controller-values` | `CONTROLLER_VALUES` | `{}` | Valeurs calculées par l'opérateur. |
| `--agent-image` | `AGENT_IMAGE` | (défaut compilé) | Image de l'agent. |

## `agent crdgen`

Génère les manifestes CRD à partir des types Rust. Sert à régénérer
[`deploy/crd/crd.yaml`](../../deploy/crd/crd.yaml).

## Codes de sortie

- `0` : succès
- `1` : erreur d'exécution (ou lint avec erreurs)
- `2` : lint avec warnings uniquement / échec de génération de CRD
- `3` : `agent package test` en échec (une assertion a échoué, ou le lint du rendu a rapporté un constat de sévérité `error`)
