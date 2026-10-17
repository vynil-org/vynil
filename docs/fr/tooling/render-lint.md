# Lint du rendu

Le lint du rendu s'exécute à la fin de chaque exécution de `agent package test`. Il remet les objets
produits par le test à un script de lint qui appartient à la **distribution**, puis replie les
constats de ce script dans le verdict ordinaire du test.

Vynil fournit le moteur et les types de données. Il n'embarque aucune règle : c'est le script qui
décide de ce qui est vérifié, avec quelle sévérité, et de ce qui est exclu. Sans script de lint, le
lint du rendu ne tourne pas et `agent package test` se comporte exactement comme avant.

> **Statut.** La bibliothèque `render_lint` porte 60 des 63 vérifications de kube-linter v0.8.3. Les
> trois autres ne sont pas portées, chacune pour une raison consignée : `scc-deny-privileged-container`
> dépend d'une CRD tierce (`SecurityContextConstraints` d'OpenShift) absente de la liste d'objets ;
> `sorted-keys` a besoin de l'ordre des clés du document source, perdu avant que les objets n'atteignent
> le script ; `schema-validation` est couverte par `render_lint_schema`, composée séparément. La fonction
> `checks()` de la bibliothèque fait foi.

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

Les deux variables suivent les autres variables de répertoire de l'agent (`PACKAGE_DIRECTORY`).
`SCHEMAS_DIRECTORY` seule, par exemple exportée par un job de CI qui ne pose pas `--lint-dir`, est une erreur
d'usage de clap à cause du `requires` : poser `LINT_DIRECTORY` avec elle.

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
| `lint(objs, ctx)` | Une fois par test, après son install et ses asserts | `objs` : carte de copies des objets créés par le test, par section (voir ci-dessous) ; `ctx` : contexte du test | tableau de constats |
| `lint_suite(ctx)` | Une fois par invocation, après la suite | `ctx` : contexte de suite | tableau de constats |

`lint` n'est pas appelée pour un test dont l'étape `value_script` ou l'install a échoué : l'échec est
déjà signalé sur le test, et un rendu partiel produirait des constats trompeurs.

### La carte `objs`

`objs` est une carte aux clés fixes. Chaque clé est toujours présente ; une clé sans objet porte un
tableau vide, si bien qu'un script n'a jamais à tester l'absence d'une clé.

| Clé | Contient |
|---|---|
| `all` | Tous les objets créés par le test, dans l'ordre de création |
| `crds`, `systems`, `befores`, `vitals`, `scalables`, `others`, `posts` | Les objets que le paquet a déclarés dans cette section |
| `unsectioned` | Les objets qu'aucune section ne déclare, dans l'ordre de `all` |

Une section est ce que le paquet a lui-même déclaré pendant l'installation, en appelant
`instance.set_status_<section>` (par exemple `set_status_scalables`). La déclaration est relue dans le
`status` de l'instance après l'install. Le lint ne déduit jamais une section du `kind` de l'objet : un
`Deployment` que le paquet n'a pas déclaré comme scalable est dans `unsectioned`, pas dans `scalables`.

Une entrée désigne un objet quand `kind`, `name` et `namespace` coïncident (un `namespace` absent doit l'être
des deux côtés), et `apiVersion` quand l'entrée en porte un. La section `crds` ne liste que des noms de
CRD. Un objet désigné par plusieurs sections figure dans chacune d'elles. Si le `status` ne peut pas être
lu, toutes les sections sont vides et tous les objets sont dans `unsectioned` ; le test n'échoue pas pour
autant.

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

`lint_suite` s'exécute aussi avec `--test-name` : `executed` ne contient alors que ce test. Une règle qui exige
quelque chose de *chaque* test doit comparer `executed` à `tests`, et ne pas supposer que toute la suite a
tourné.

## Forme d'un constat

Un constat est une carte renvoyée dans le tableau. Champs :

| Champ | Type | Obligatoire | Sens |
|---|---|---|---|
| `rule` | chaîne non vide | oui | Nom de la règle. Conservé tel quel dans le verdict, `/` compris (`schema/invalid`). Les noms `execution`, `invalid-finding` et `sections` sont réservés : le moteur produit lui-même les lignes `lint:execution`, `lint:invalid-finding` et `lint:sections`, et un constat qui en utilise un est invalide. |
| `severity` | `"error"` ou `"warn"` | oui | Effet sur le verdict (voir ci-dessous). |
| `object` | carte `{apiVersion, kind, namespace?, name}` | oui | Objet concerné. `apiVersion`, `kind` et `name` sont des chaînes non vides ; `namespace` est optionnel, et un `namespace` vide vaut absent. |
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

La bibliothèque est dérivée de [kube-linter](https://github.com/stackrox/kube-linter) (licence Apache
2.0) : les noms, paramètres, valeurs par défaut et comportements mesurés de ses vérifications sont ceux de
l'outil de référence, portés en Rhai pour s'exécuter sur les objets d'un rendu de test sans cluster. Le
`NOTICE` du dépôt en porte l'attribution. La bibliothèque est faite pour être lue autant qu'appelée : une
distribution qui écrit ses propres règles peut s'en inspirer, et chaque module documente le comportement
qu'il reproduit.

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
| `render_lint::api_group_of(api_version)` | Groupe d'API d'une `apiVersion` : la partie avant `/` (`apps` pour `apps/v1`), la chaîne vide pour `v1` (groupe du noyau), `()` pour une version sans `/` autre que `v1`. |
| `render_lint::kind_of_group(obj, kinds, group)` | `true` quand l'objet a l'un des `kinds` dans le groupe d'API `group` (`""` pour le noyau). La seule façon pour une vérification de reconnaître un objet : un kind seul ne suffit jamais, un `Deployment` d'un autre groupe n'est pas une charge de travail `apps`. |
| `render_lint::pod_templates(obj)` | Tableau de `#{pod, path}` pour les objets qui embarquent un gabarit de pod. `path` vaut `spec.template`, `spec.jobTemplate.spec.template`, ou une chaîne vide pour un `Pod`. Vide pour les autres objets. |
| `render_lint::containers(entry)` | Tableau de `#{container, path, init}` pour une entrée de `pod_templates` : conteneurs puis conteneurs d'init, avec des chemins complets tels que `spec.template.spec.containers[0]`. |
| `render_lint::suppression_reason(obj, rule, opts)` | Valeur de l'annotation `{ignore_prefix}{rule}` de l'objet, ou `()` si elle est absente ou si `ignore_prefix` n'est pas donné. |
| `render_lint::apply_suppressions(findings, objs, opts)` | Marque supprimés les constats dont l'objet porte l'annotation. Renseigne `reason` (si non vide) et `origin`. |
| `render_lint::all(objs, ctx, opts)` | Exécute les vérifications activées sur tous les objets, et applique les suppressions. Ordre : objets dans l'ordre, puis vérifications par ordre alphabétique de leur nom. Sans option, seules les vérifications que kube-linter active par défaut s'exécutent. |
| `render_lint::checks()` | Tableau trié des noms des vérifications fournies, au nommage kube-linter (tirets). |
| `render_lint::default_checks()` | Tableau trié des vérifications que kube-linter active par défaut (31 sur 60). |
| `render_lint::index(objs)` | L'index d'une liste d'objets (voir ci-dessous). Utile seulement pour appeler une vérification directement. |

Chaque vérification est aussi une fonction `{name}(obj, idx, ctx, params)`, où les tirets du nom
kube-linter deviennent des soulignés. Elle retourne un tableau de constats, vide s'il n'y a rien à
signaler, et un tableau vide pour les kinds qu'elle ne concerne pas. Le `rule` d'un constat est le nom
kube-linter, tirets compris, pour qu'une liste `exclude` ou une annotation de suppression fonctionne d'un
outil à l'autre.

`idx` est l'index de la liste d'objets, pas la liste elle-même. Rhai passe toute valeur par copie : donner
la liste entière à chacune des soixante vérifications pour chaque objet la copie soixante fois par objet,
ce qui est quadratique. `render_lint::all` construit l'index une fois par `render_lint::index(objs)`, une
carte de poignées partagées que les vérifications qui rapprochent plusieurs objets (un Service de ses pods,
une liaison de son rôle, une référence d'env d'un Secret) interrogent au lieu de parcourir la liste. Un
script qui appelle une vérification directement passe `render_lint::index(objs.all)`. L'argument `ctx` est
réservé : aucune vérification portée ne le lit et `all` passe `()`.

Le coût de `render_lint::all` est donc linéaire en nombre d'objets.

### Options de `render_lint::all`

Toutes les clés sont facultatives.

| Clé | Défaut | Sens |
|---|---|---|
| `exclude` | aucun | Noms des vérifications à ne pas exécuter. Toujours appliqué. |
| `only` | absent | Quand présent, exactement ces vérifications s'exécutent, y compris celles que kube-linter n'active pas par défaut. |
| `include_optional` | `false` | Exécute aussi les vérifications que kube-linter n'active pas par défaut (`required-label-owner`, `minimum-three-replicas`, `no-node-affinity`, `use-namespace`…). |
| `severity` | `"error"` pour chaque vérification | Carte nom de vérification vers `"error"` ou `"warn"`. Appliquée après l'exécution de la vérification. |
| `params` | vide | Carte nom de vérification vers une carte de paramètres, avec les noms de paramètres de kube-linter. |
| `ignore_prefix` | absent | Préfixe des annotations de suppression. Sans lui, aucune suppression n'a lieu. |
| `require_reason` | `false` | Quand `true`, une suppression dont la valeur d'annotation est vide n'est pas appliquée. Le constat reste, et un constat `suppression-without-reason` de sévérité `error` est ajouté, une fois par objet et par règle, nommant l'annotation. Quand `false`, une annotation vide supprime avec la raison `(aucune raison donnée)` : le moteur refuse une suppression sans raison. Mettre `true` dans une distribution qui veut chaque suppression justifiée ; le défaut garde seulement l'inventaire honnête. |

Sans option, `render_lint::all` exécute les vérifications que kube-linter active par défaut, ce
qu'attendra une distribution qui recopie l'exemple. `include_optional: true` exécute les soixante.

**Une option qui ne peut pas s'appliquer est une erreur, pas un non-événement.** Le script échoue avec un
message qui nomme l'option, et le test rapporte une ligne `lint:execution`, pour : une clé d'option
inconnue ; un nom de vérification absent de `render_lint::checks()` dans `exclude`, `only`, `severity` ou
`params` (`exclude: ["latest_tag"]` avec un souligné n'exclurait sinon rien) ; une sévérité autre que
`"error"` ou `"warn"` ; un paramètre que la vérification n'a pas ; et une valeur de paramètre de mauvais type
ou hors de son énumération (`requirementsType: "requests"` au lieu de `"request"`, une chaîne là où une liste
est attendue).

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
import "render_lint_schema" as schema;

// Règle maison : tout Deployment porte un label `team`.
fn team_label(objs) {
    let findings = [];
    for obj in objs {
        if render_lint::kind_of_group(obj, ["Deployment"], "apps") {
            let labels = if "labels" in obj.metadata { obj.metadata.labels } else { #{} };
            if !("team" in labels) {
                findings.push(render_lint::finding("team-label", "error", obj, "label `team` is required"));
            }
        }
    }
    findings
}

fn lint(objs, ctx) {
    let findings = team_label(objs.all);
    findings += render_lint::all(objs.all, ctx, #{
        exclude: ["latest-tag"],
        severity: #{ "no-read-only-root-fs": "warn" },
        ignore_prefix: "ignore-check.example/",
        require_reason: true
    });
    // Les vérifications ci-dessus restent muettes sur un objet mal formé (elles ne retournent rien) : c'est la validation de schéma qui le signale.
    for obj in objs.all {
        findings += schema::check(obj);
    }
    findings
}
```

Les vérifications de `render_lint::all` ne retournent aucun constat pour un objet dont elles ne savent pas
lire la forme (une `spec` qui n'est pas une carte, un `env` du mauvais type). C'est le comportement de
la bibliothèque, et il n'est sûr qu'à côté de la validation de schéma, qui signale justement ces
formes : `render_lint::all` ne l'appelle pas, l'exemple appelle donc les deux.

Exécution :

```bash
agent package test -p ./my-package --all --lint-dir ./distribution/lint
```

Un Deployment sans label `team` produit une ligne en échec `lint:team-label`. Une annotation
`ignore-check.example/team-label: "legacy"` posée sur ce Deployment ne la supprime pas, parce que la règle
maison construit son constat directement et ne passe pas par `render_lint::apply_suppressions`. Pour rendre une
règle maison supprimable, faire passer ses constats par `render_lint::apply_suppressions(findings, objs.all, opts)`.

## Limites

- Trois vérifications de kube-linter ne sont pas portées (voir la note de statut).
- Les valeurs du budget d'opérations et sa configuration ne sont pas encore fixées. Le budget est
  proportionnel au nombre d'objets, et `render_lint::all` coûte environ seize mille opérations par objet avec
  les soixante vérifications.
- Un constat `warn` n'a pas de champ de sévérité dans le verdict.
- `env-value-from` résout une référence à un Secret ou à un ConfigMap par kind et nom, **dans n'importe quel
  namespace**, comme kube-linter : la fixture `env-value-from` référence un Secret d'un autre namespace et
  l'outil ne signale rien. Un Secret de même nom dans un autre namespace satisfait donc la référence.
- Une quantité de ressource hors de la grammaire de Kubernetes (un signe, une unité inconnue, du texte
  libre) est sautée par `unset-cpu-requirements` et `unset-memory-requirements`. Les formes à exposant
  (`129e6`) et les suffixes `P`, `E`, `Pi`, `Ei`, `n`, `u` sont lus. Les formes étendues sont testées contre
  la définition de la `Quantity` de Kubernetes, pas contre l'outil de référence.
- kube-linter plante (SIGSEGV) sur une option `dnsConfig` dont le nom est la clé cherchée mais qui n'a pas de
  valeur. Ce cas ne se confronte pas à l'oracle, il n'a donc pas d'`expected.json` ; la bibliothèque le signale
  sans planter. C'est la seule divergence connue consignée ici.
