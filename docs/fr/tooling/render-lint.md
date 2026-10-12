# Lint du rendu

Le lint du rendu s'exécute à la fin de chaque exécution de `agent package test`. Il remet les objets
produits par le test à un script de lint qui appartient à la **distribution**, puis replie les
constats de ce script dans le verdict ordinaire du test.

Vynil fournit le moteur et les types de données. Il n'embarque aucune règle : c'est le script qui
décide de ce qui est vérifié, avec quelle sévérité, et de ce qui est exclu. Sans script de lint, le
lint du rendu ne tourne pas et `agent package test` se comporte exactement comme avant.

> **Statut.** Le contrat ci-dessous est celui fixé par les spécifications. La bibliothèque de
> vérifications (`render_lint`) est spécifiée, avec des vérifications inspirées de kube-linter, mais
> son catalogue n'est pas encore livré en entier : ne pas supposer qu'une vérification donnée existe
> tant qu'elle n'est pas listée par la fonction `checks()` de la bibliothèque.

## Activation

Le lint du rendu s'active par `--lint-dir` sur `agent package test` (voir [Tests de paquet](test.md#lint-du-rendu)
et [Référence CLI](../cli.md#agent-package-test)) :

```bash
agent package test -p ./my-package --all \
  --lint-dir ./distribution/lint \
  --schemas-dir ./distribution/schemas
```

| Flag | Variable d'environnement | Rôle |
|---|---|---|
| `--lint-dir <dir>` | `LINT_DIRECTORY` | Répertoire contenant `lint.rhai`, le point d'entrée du script de la distribution. |
| `--schemas-dir <dir>` | `SCHEMAS_DIRECTORY` | Répertoire de JSON Schemas et de CRD utilisé par les fonctions `schema_*`. Exige `--lint-dir`. |

## Le répertoire de lint

`lint.rhai` est le point d'entrée fixe. Les modules qu'il importe sont résolus dans cet ordre :

1. le répertoire de lint lui-même ;
2. `{script-dir}/lib`, où `script-dir` est le flag `--script-dir` (défaut `./agent/scripts`).

Les bibliothèques `render_lint` et `render_lint_schema` sont donc prises dans le répertoire de scripts
de l'agent, et non dans la distribution.

Un `lint.rhai` absent, un script qui ne compile pas, ou un script qui ne définit ni `lint` ni
`lint_suite` arrêtent la commande avec le code de sortie 3, avant toute sortie.

## Points d'entrée

Le script peut définir une de ces fonctions ou les deux. Une fonction non définie est sautée.

| Fonction | Appelée | Arguments | Retourne |
|---|---|---|---|
| `lint(objs, ctx)` | Une fois par test, après son install et ses asserts | `objs` : copie des objets créés par le test, dans l'ordre de création ; `ctx` : contexte du test | tableau de constats |
| `lint_suite(ctx)` | Une fois par invocation, après la suite | `ctx` : contexte de suite | tableau de constats |

`lint` n'est pas appelée pour un test dont l'étape `value_script` ou l'install a échoué : l'échec est
déjà signalé sur le test, et un rendu partiel produirait des constats trompeurs.

Chaque appel s'exécute dans un moteur Rhai neuf, sans état partagé entre tests. Le script reçoit des
copies : il ne peut pas modifier les objets du test. Les fonctions disponibles sont les fonctions de
manipulation de données du moteur de base et les fonctions `schema_*`. Aucune fonction Kubernetes, HTTP,
S3, shell ou fichier n'est enregistrée : un script qui en appelle une échoue avec une erreur de fonction
inconnue.

L'exécution est bornée par une limite d'opérations pour qu'un script qui boucle ne bloque pas la suite.
La valeur de cette limite n'est pas encore fixée par le contrat.

### Erreurs du script

Chacun des cas suivants devient une ligne en échec nommée `lint:execution`, dont le message est l'erreur,
et ne produit aucune suppression pour cet appel :

- une erreur d'exécution Rhai, y compris l'appel à une fonction inconnue ;
- l'atteinte de la limite d'opérations ;
- une valeur de retour qui n'est pas un tableau (une carte, par exemple).

Les scripts sont déterministes : mêmes objets et mêmes contextes donnent les mêmes lignes.

## Contexte de test : `ctx` dans `lint`

`ctx` est en lecture seule et ne contient que des données sérialisables. Il est construit à partir du
test résolu.

```text
ctx
├── package
│   ├── name
│   ├── type            tenant | service | system
│   ├── category
│   ├── description
│   ├── app_version     absent si non défini
│   ├── features
│   ├── options         options déclarées dans package.yaml
│   ├── images          tel que dans package.yaml
│   └── resources       tel que dans package.yaml
├── instance
│   ├── name
│   ├── namespace
│   ├── appslug         appslug du paquet et de l'instance
│   └── options         options fournies par le test, non résolues
└── test
    ├── name
    ├── description     absent si non défini
    ├── testsets        [{name, variables}] pour chaque testSet référencé
    └── definition      Test résolu : apiVersion, kind, metadata, instance, mocks, asserts
```

`test.definition.asserts` contient les asserts du test suivis de ceux de ses testSets.

`instance.options` ne contient pas les valeurs par défaut des options du paquet : une règle qui en a
besoin combine `package.options` et `instance.options`.

## Contexte de suite : `ctx` dans `lint_suite`

```text
ctx
├── package             même forme que dans le contexte de test
└── suite
    ├── tests           carte : nom de test -> document Test chargé, non rendu
    ├── testsets        carte : nom de testSet -> document TestSet chargé
    └── executed        noms des tests exécutés par cette invocation, par ordre croissant
```

## Forme d'un constat

Un constat est une carte renvoyée dans le tableau. Champs :

| Champ | Type | Obligatoire | Sens |
|---|---|---|---|
| `rule` | chaîne non vide | oui | Nom de la règle. Conservé tel quel dans le verdict, `/` compris (`schema/invalid`). |
| `severity` | `"error"` ou `"warn"` | oui | Effet sur le verdict (voir ci-dessous). |
| `object` | carte `{apiVersion, kind, namespace?, name}` | oui | Objet concerné. `apiVersion`, `kind` et `name` sont des chaînes non vides ; `namespace` est optionnel. |
| `message` | chaîne non vide | oui | Message affiché dans le verdict. |
| `path` | chaîne | non | Chemin pointé dans l'objet, par exemple `spec.template.spec.containers[0].image`. |
| `source` | chaîne | non | Information libre sur l'origine. |
| `suppressed` | booléen | non | Défaut `false`. Un constat supprimé est inventorié, pas rapporté. |
| `reason` | chaîne | non | Raison donnée à une suppression. |
| `origin` | chaîne | non | Provenance de la suppression, par exemple `annotation:{prefix}{rule}`. |

Les clés inconnues sont ignorées.

Un élément qui ne respecte pas cette forme n'est **jamais écarté**. Il devient une ligne en échec
nommée `lint:invalid-finding`, dont le message nomme son index (`finding #0: ...`). Un script mal écrit
fait donc passer le test au rouge au lieu de réussir en silence.

L'ordre du tableau renvoyé par le script est conservé : rien n'est trié ni dédupliqué.

## Verdict

Chaque constat devient une ligne du verdict du test :

| Cas | Nom de la ligne | Réussie | Message |
|---|---|---|---|
| `error`, non supprimé | `lint:{rule}` | non | `message`, suivi de ` [{path}]` quand `path` est présent |
| `warn`, non supprimé | `lint:{rule}` | oui | `warning: ` suivi du message (et du chemin) |
| supprimé (toute sévérité) | aucune | — | inventorié, voir ci-dessous |
| élément invalide | `lint:invalid-finding` | non | description de l'invalidité |
| erreur du script | `lint:execution` | non | l'erreur |

La description d'une ligne est `{kind} {namespace}/{name}`, ou `{kind} {name}` pour un objet non
namespacé. Pour le hook de suite, les lignes appartiennent à un enregistrement nommé `(suite)`.

Une ligne en échec compte comme n'importe quel assert en échec : le résumé, le rapport JUnit et le code
de sortie 3 s'appliquent sans changement. Un constat `warn` ne fait jamais échouer le test.

Le modèle de verdict n'ayant pas de champ de sévérité, un avertissement n'est exprimé que par sa ligne
réussie et son préfixe `warning: `. L'ajout d'un champ de sévérité explicite est une question ouverte
dans les spécifications.

## Suppressions

Un constat marqué `suppressed: true` ne fait jamais échouer un test. Il est conservé dans un inventaire,
pour que chaque exception reste visible dans les rapports.

Le moteur ne décide pas de ce qui est supprimé. La bibliothèque `render_lint` marque les constats comme
supprimés à partir d'annotations posées sur l'objet (voir plus bas), mais un script peut aussi positionner
`suppressed` lui-même.

L'inventaire n'est rendu que s'il n'est pas vide :

- **text** : après la ligne `Results:`, une ligne `Suppressed: {n}`, puis une ligne par suppression :
  `  [SKIP] {test}: {rule} {kind} {namespace}/{name}: {reason}`, avec `(no reason)` quand la raison est
  absente ;
- **json** : après `summary`, un tableau `suppressions`. Chaque entrée a les clés `test`, `rule`, `kind`,
  `namespace`, `name`, `path`, `reason`, `origin` ; les champs absents valent `null` ;
- **JUnit** : un élément `<system-out>` sur le `<testsuite>` du test concerné, une suppression par ligne.
  Aucune autre suite ne porte de `<system-out>` pour les suppressions.

Une suppression ne modifie ni les compteurs ni l'état de réussite de l'exécution.

## Les fonctions `schema_*`

Ces fonctions ne sont enregistrées dans le moteur du script de lint que si `--lint-dir` est donné.

| Fonction | Retourne |
|---|---|
| `schema_known(obj)` | `true` si un schéma existe pour l'`apiVersion` et le `kind` de l'objet. Un kind inconnu est un fait, pas une erreur. |
| `schema_validate(obj)` | tableau de `#{path, message}`, vide si l'objet est conforme. Équivaut à `strict: true`. |
| `schema_validate(obj, #{strict: bool})` | idem, avec le mode strict réglé. `strict: false` désactive seulement le mode strict ; la normalisation du schéma reste appliquée. |
| `schema_native_version()` | version des schémas Kubernetes natifs embarqués dans le binaire, sous forme de chaîne. |

Le `path` retourné par `schema_validate` est un pointeur JSON (par exemple `/spec/template/spec/containers/0`,
vide à la racine). La conversion en notation pointée est faite par la bibliothèque `render_lint_schema`.

Les schémas viennent de trois sources, résolues dans cet ordre :

1. les CRD du paquet, lues dans `<package-dir>/crds` ;
2. le répertoire de schémas de la distribution (`--schemas-dir`) ;
3. la table des schémas Kubernetes natifs embarquée dans le binaire.

Le répertoire de schémas contient :

- des fichiers JSON Schema nommés `<groupe>/<Kind>_<version>.json`, avec la casse exacte du Kind, par
  exemple `apps/Deployment_v1.json` ; le groupe vide s'écrit avec le dossier littéral `core`
  (`core/Pod_v1.json`) ;
- des fichiers YAML contenant des documents `CustomResourceDefinition`.

Les autres fichiers sont ignorés. Un répertoire absent ou vide n'ajoute aucune source ; la table native
reste active. Un fichier JSON non analysable arrête la commande, et l'erreur nomme ce fichier.

## La bibliothèque `render_lint`

Le script de la distribution compose la bibliothèque par `import`. L'alias est le nom du module
(`render_lint`), jamais `lint` : c'est le nom du point d'entrée `fn lint(objs, ctx)` du script.

```rhai
import "render_lint" as render_lint;
```

### Fonctions exposées

| Fonction | Rôle |
|---|---|
| `render_lint::finding(rule, severity, obj, message)` et `render_lint::finding(rule, severity, obj, path, message)` | Construit une carte constat. |
| `render_lint::object_ref(obj)` | La carte `object` d'un objet. |
| `render_lint::pod_templates(obj)` | Tableau de `#{pod, path}` pour les objets qui embarquent un gabarit de pod. `path` vaut `spec.template`, `spec.jobTemplate.spec.template`, ou une chaîne vide pour un `Pod`. Vide pour les autres objets. |
| `render_lint::containers(entry)` | Tableau de `#{container, path, init}` pour une entrée de `pod_templates` : conteneurs puis conteneurs d'init, avec des chemins complets tels que `spec.template.spec.containers[0]`. |
| `render_lint::suppression_reason(obj, rule, opts)` | Valeur de l'annotation `{ignore_prefix}{rule}` de l'objet, ou `()` si elle est absente ou si `ignore_prefix` n'est pas donné. |
| `render_lint::apply_suppressions(findings, objs, opts)` | Marque supprimés les constats dont l'objet porte l'annotation. Renseigne `reason` (si non vide) et `origin`. |
| `render_lint::all(objs, ctx, opts)` | Exécute toutes les vérifications activées sur tous les objets, et applique les suppressions. Ordre : objets dans l'ordre, puis vérifications par ordre alphabétique de leur nom. |
| `render_lint::checks()` | Tableau trié des noms des vérifications fournies, au nommage kube-linter (tirets). |

Chaque vérification est aussi une fonction `{name}(obj, objs, ctx, params)`, où les tirets du nom
kube-linter deviennent des soulignés. Elle retourne un tableau de constats, vide s'il n'y a rien à
signaler, et un tableau vide pour les kinds qu'elle ne concerne pas. Le `rule` d'un constat est le nom
kube-linter, tirets compris, pour qu'une liste `exclude` ou une annotation de suppression fonctionne d'un
outil à l'autre.

### Options de `render_lint::all`

Toutes les clés sont facultatives.

| Clé | Défaut | Sens |
|---|---|---|
| `exclude` | aucun | Noms des vérifications à ne pas exécuter. |
| `only` | absent | Quand présent, seules ces vérifications s'exécutent. |
| `severity` | `"error"` pour chaque vérification | Carte nom de vérification vers `"error"` ou `"warn"`. Appliquée après l'exécution de la vérification. |
| `params` | vide | Carte nom de vérification vers une carte de paramètres, avec les noms de paramètres de kube-linter. |
| `ignore_prefix` | absent | Préfixe des annotations de suppression. Sans lui, aucune suppression n'a lieu. |
| `require_reason` | `false` | Quand `true`, une suppression dont la valeur d'annotation est vide n'est pas appliquée. Le constat reste, et un constat `suppression-without-reason` de sévérité `error` est ajouté, une fois par objet et par règle, nommant l'annotation. |

`render_lint::all` se contente d'assembler les vérifications. La bibliothèque ne sélectionne rien par défaut :
aucune vérification ne tourne tant que le script de la distribution ne l'appelle pas.

### Validation de schéma

`render_lint_schema` est séparée de `render_lint::all` : la validation de schéma est opt-in.

```rhai
import "render_lint_schema" as schema;
```

`schema.check(obj)` et `schema.check(obj, opts)` retournent un tableau de constats pour un objet :

| Clé | Défaut | Sens |
|---|---|---|
| `strict` | `true` | Un champ non déclaré est une erreur. `false` désactive seulement le mode strict. |
| `unknown` | `"warn"` | Sévérité de `schema/unknown-kind` quand aucun schéma n'existe : `"warn"`, `"error"` ou `"ignore"` (aucun constat). |
| `severity` | `"error"` | Sévérité des constats `schema/invalid`. |

Un objet invalide produit un constat `schema/invalid` par erreur de validation, avec le `path` pointé et le
message du moteur. Un objet sans `apiVersion` ou `kind` reçoit un constat `schema/invalid` à la racine.

## Exemple : `lint.rhai`

Un script de distribution minimal, avec une règle maison et l'appel à la bibliothèque :

```rhai
import "render_lint" as render_lint;

// Règle maison : tout Deployment porte un label `team`.
fn team_label(objs) {
    let findings = [];
    for obj in objs {
        if obj.kind == "Deployment" {
            let labels = if "labels" in obj.metadata { obj.metadata.labels } else { #{} };
            if !("team" in labels) {
                findings.push(render_lint::finding("team-label", "error", obj, "label `team` is required"));
            }
        }
    }
    findings
}

fn lint(objs, ctx) {
    let findings = team_label(objs);
    findings += render_lint::all(objs, ctx, #{
        exclude: ["latest-tag"],
        severity: #{ "no-read-only-root-fs": "warn" },
        ignore_prefix: "ignore-check.example/",
        require_reason: true
    });
    findings
}
```

Exécution :

```bash
agent package test -p ./my-package --all --lint-dir ./distribution/lint
```

Un Deployment sans label `team` produit une ligne en échec `lint:team-label`. Une annotation
`ignore-check.example/team-label: "legacy"` posée sur ce Deployment ne la supprime pas, parce que la règle
maison construit son constat directement et ne passe pas par `render_lint::apply_suppressions`. Pour rendre une
règle maison supprimable, faire passer ses constats par `render_lint::apply_suppressions(findings, objs, opts)`.

## Limites

- Le catalogue de vérifications de la bibliothèque n'est pas encore livré en entier (voir la note de statut).
- La valeur de la limite d'opérations et sa configuration ne sont pas encore fixées.
- Un constat `warn` n'a pas de champ de sévérité dans le verdict.
