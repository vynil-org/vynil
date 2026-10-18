# Tests de paquet

Vynil distingue deux niveaux de test : les **tests d'un paquet** (`agent package test`,
écrits par l'auteur du paquet) et les **tests de régression internes** (suite Rust de
l'agent, garantissant la stabilité du moteur Rhai).

## Tests d'un paquet — `agent package test`

Exécute des scénarios définis dans `<package-dir>/tests/` avec un moteur Rhai réel et des
**mocks K8s/HTTP/OCI** : aucun cluster requis. Le rendu des templates et les hooks du cycle
de vie sont exécutés, et des assertions valident le résultat.

```bash
# tous les tests
agent package test -p ./my-package --all

# un test précis, avec dump du rendu
agent package test -p ./my-package --test-name install-default \
  --template-output-filename rendered.yaml

# rapport JUnit pour la CI
agent package test -p ./my-package --all --format json \
  --junit-output-filename results.xml
```

| Option | Rôle |
|---|---|
| `--test-name <name>` | Exécute un seul test. |
| `--all` | Exécute tous les tests. |
| `--testsets-dir <dir>` | Répertoire additionnel de jeux de tests. |
| `--format text\|json` | Format de sortie. |
| `--junit-output-filename <file>` | Rapport JUnit XML. |
| `--template-output-filename <file>` | Dump du rendu (test unique uniquement). |
| `--lint-dir <dir>` | Répertoire du script de lint de la distribution (`lint.rhai`). Active le lint du rendu. |
| `--schemas-dir <dir>` | Répertoire de schémas de la distribution. Exige `--lint-dir`. |

Le répertoire `tests/` doit exister, sinon l'agent renvoie `MissingTestDirectory`.

### Ce qu'un test peut faire

- fournir des **options** d'instance et un **contexte** (cluster/tenant) ;
- déclarer des **mocks** : objets K8s présents, réponses HTTP, manifestes OCI ;
- exécuter un flow (`context → install`, `context → delete`, …) ;
- **asserter** sur les objets créés, le statut, ou le rendu des templates.

### Variables des assertions

Les champs `selector` et `value` des assertions sont des templates Handlebars rendus après
l'installation, sur le même `context` que celui que `ctx::run()` a transmis aux templates
et aux scripts du paquet. Les mocks sont rendus plus tôt, avant tout script, et ne voient
que `package` et `instance`.

| Variable | Source |
|----------|--------|
| `values`, `defaults` | Options fusionnées avec les défauts du paquet, défauts bruts |
| `cluster` | `agent.yaml` et le cluster mocké (storage classes, nodes…) |
| `tenant` | Nom et namespaces du tenant (paquets tenant) |
| `controller` | Sortie du `value_script` du paquet |
| `instance` | `name`, `namespace`, `appslug`, `options` du test, plus les clés de l'instance du contexte (`labels`, `selector`…) |
| `package` | La définition du paquet (`package.metadata.name`…) |
| `var` | Variables du testSet référençant (asserts de testSet uniquement) |

Une variable absente du contexte fait échouer le test avec une ligne `execution` qui la nomme.

### Bonnes pratiques

- **Tests prévisibles** : ne pas asserter sur des champs dépendants de la version amont
  (tags d'images de conteneurs, `app_version`) — ils casseraient à chaque
  `agent package update`. Asserter plutôt sur la structure, les noms et les valeurs
  dérivées des options.
- **Un jeu de test par posture** : un `default.yaml` minimal, puis des jeux dédiés aux
  variantes significatives (HA activé, option majeure activée…), plutôt qu'un test unique
  qui mélange tout.
- **Utiliser les placeholders de contexte** (`{{instance.appslug}}`,
  `{{instance.namespace}}`) dans les valeurs attendues, pour que les tests restent valides
  quel que soit le nom de l'instance.

## Lint du rendu

Quand `--lint-dir` est donné, le script de lint de la distribution s'exécute à la fin de chaque test,
sur les objets créés par ce test. C'est là qu'une distribution fait appliquer ses propres règles à un
paquet ; Vynil n'en embarque aucune. Le contrat complet du script (points d'entrée, contexte, forme
d'un constat, suppressions, fonctions `schema_*`) est décrit dans
[Lint du rendu](render-lint.md).

```bash
agent package test -p ./my-package --all \
  --lint-dir ./distribution/lint --schemas-dir ./distribution/schemas
```

- **Quand il s'exécute** : après l'install et les asserts d'un test, et seulement si ni l'étape
  `value_script` ni l'étape d'install n'ont échoué. Le hook de suite `lint_suite` s'exécute une fois
  par invocation, dans les modes `--all` et `--test-name`.
- **Effet sur le verdict** : chaque constat `error` est un assert en échec nommé `lint:{rule}` ; le
  test échoue avec le code de sortie 3, comme tout assert en échec. Un constat `warn` est une ligne
  réussie préfixée de `warning: ` et ne fait jamais échouer l'exécution. Une erreur du script donne une
  ligne en échec `lint:execution`, et un constat invalide une ligne en échec `lint:invalid-finding`.
- **Sans `--lint-dir`** : rien ne change. Aucune ligne de lint ni aucune suppression n'est produite.

### Suppressions

Un constat marqué comme supprimé ne fait jamais échouer un test. Il est porté par un inventaire, rendu
seulement quand il n'est pas vide :

- **text** : une ligne `Suppressed: {n}` après la ligne `Results:`, puis une ligne `[SKIP]` par
  suppression, avec son test, sa règle, son objet et sa raison ;
- **json** : un tableau `suppressions` après `summary`, un objet par suppression avec `test`, `rule`,
  `kind`, `namespace`, `name`, `path`, `reason` et `origin` (les champs absents valent `null`) ;
- **JUnit** : un élément `<system-out>` sur le `<testsuite>` du test concerné, une suppression par ligne.

## Tests de régression internes (`agent/tests/rhai_*.rs`)

La suite Rust exécute les scripts Rhai *de l'agent* (pas ceux d'un paquet) avec un moteur
réel et des mocks. Elle sert de filet de sécurité lors des montées de version de Rhai, en
capturant les changements de sémantique des fonctions de manipulation de chaînes et de
collections (`.filter()`, `.replace()`, `.reduce()`, closures).

- **niveau unitaire** : fonctions isolées de `agent/scripts/lib/` (storage_class, wait,
  install_from_dir, gen_package, backup_context, resolv_service…) ;
- **niveau intégration** : flows de cycle de vie service/install et service/delete de bout
  en bout avec mocks K8s, validant que les scripts `lib/` s'assemblent correctement.

```bash
# lancer toute la suite
cargo test -p agent

# une famille de tests
cargo test -p agent --test rhai_build
```

## En CI

Enchaînement recommandé pour un paquet :

```bash
agent package lint -p ./pkg --format junit --junit-output-filename lint.xml
agent package test -p ./pkg --all --format json --junit-output-filename test.xml
agent package build -p ./pkg --signing-key "$SIGNING_KEY"   # publication signée
```
