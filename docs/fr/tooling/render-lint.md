# Lint du rendu

Le lint du rendu s'exécute à la fin de chaque exécution de `agent package test`. Il remet les objets
produits par le test à un script de lint qui appartient à la **distribution**, puis replie les
constats de ce script dans le verdict ordinaire du test.

Vynil fournit le moteur et les types de données. Il n'embarque aucune règle : c'est le script qui
décide de ce qui est vérifié, avec quelle sévérité, et de ce qui est exclu. Sans script de lint, le
lint du rendu ne tourne pas et `agent package test` se comporte exactement comme avant.

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

Le répertoire de lint passe en premier : un module du même nom posé là remplace le module standard de
l'agent, comme un paquet peut surcharger un fichier fourni par l'agent. Une distribution qui masque un
module standard par inadvertance change donc ce que son propre script importe.

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
lu, toutes les sections sont vides, tous les objets sont dans `unsectioned`, et une ligne en échec
`lint:sections` est ajoutée : une règle du type « pour chaque objet de `objs.scalables` » passerait sinon
sans avoir rien vérifié. Un `status` lu avec succès mais qui ne déclare aucune section est légitime et
n'ajoute aucune ligne.

Chaque appel s'exécute dans un moteur Rhai neuf, sans état partagé entre tests. Le script reçoit des
copies : il ne peut pas modifier les objets du test. Les fonctions disponibles sont les fonctions de
manipulation de données du moteur de base et les fonctions `schema_*`. Aucune fonction Kubernetes, HTTP,
S3, shell ou fichier n'est enregistrée : un script qui en appelle une échoue avec une erreur de fonction
inconnue.

L'exécution est bornée pour qu'un script qui boucle ne bloque pas la suite : par un budget d'opérations,
une base plus une part par objet reçu, et par une limite de taille des chaînes, tableaux et cartes (un
script qui double une chaîne en boucle échoue au lieu d'épuiser la mémoire). Les valeurs du budget ne sont
pas encore fixées par le contrat.

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
| `reason` | chaîne | si `suppressed` vaut `true` | Raison donnée à une suppression. Doit alors être non vide. |
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
| `status` de l'instance illisible | `lint:sections` | non | l'erreur de lecture |

La description d'une ligne est `{kind} {namespace}/{name}`, ou `{kind} {name}` pour un objet non
namespacé. Pour le hook de suite, les lignes appartiennent à un enregistrement nommé `(suite)`.

Une ligne en échec compte comme n'importe quel assert en échec : le résumé, le rapport JUnit et le code
de sortie 3 s'appliquent sans changement. Un constat `warn` ne fait jamais échouer le test.

Le modèle de verdict n'a pas de champ de sévérité : un avertissement est une ligne réussie au préfixe
`warning: `. Le résumé texte le montre tel quel. Le rapport JUnit ajoute deux choses pour les lignes
nommées `lint:...` (un assert ordinaire garde son rapport inchangé, octet pour octet) :

- la description de la ligne (`Deployment app/web`) est écrite en `classname` du cas de test, si bien que
  dix objets en échec sur la même règle sont dix cas distincts et non dix cas `lint:{rule}` identiques ;
- une ligne réussie dont le message commence par `warning: ` est listée dans le `<system-out>` de la suite
  sous la forme `[WARN] {nom} ({description}): {message}`, après les lignes `[SKIP]` des suppressions.
  Sans cela, un lint dont tous les constats sont des avertissements paraîtrait vert et muet.

## Suppressions

Un constat marqué `suppressed: true` ne fait jamais échouer un test. Il est conservé dans un inventaire,
pour que chaque exception reste visible dans les rapports.

Le moteur ne décide pas de ce qui est supprimé : le script positionne `suppressed` lui-même, par exemple à
partir d'une annotation posée sur l'objet. Une suppression doit dire pourquoi : `suppressed: true` sans
`reason` non vide est un constat invalide (`lint:invalid-finding`), pour qu'une erreur ne puisse jamais être
réduite au silence sans trace de sa justification.

L'inventaire n'est rendu que s'il n'est pas vide :

- **text** : après la ligne `Results:`, une ligne `Suppressed: {n}`, puis une ligne par suppression :
  `  [SKIP] {test}: {rule} {kind} {namespace}/{name}: {reason}` ;
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
| `schema_validate(obj)` | tableau de `#{path, message}`, vide si l'objet est conforme **ou si aucun schéma n'est connu pour son `apiVersion` et son `kind`**. Équivaut à `strict: true`. |
| `schema_validate(obj, #{strict: bool})` | idem, avec le mode strict réglé. `strict: false` désactive seulement le mode strict ; la normalisation du schéma reste appliquée. |
| `schema_native_version()` | version des schémas Kubernetes natifs embarqués dans le binaire, sous forme de chaîne. |

Le `path` retourné par `schema_validate` est un pointeur JSON (par exemple `/spec/template/spec/containers/0`,
vide à la racine). La conversion en notation pointée est laissée au script.

Un objet de kind inconnu est donc rapporté comme conforme, et une faute de frappe dans une `apiVersion`
(`apps/v1beta` au lieu de `apps/v1`) passe sans bruit. Un script qui veut la signaler appelle d'abord
`schema_known(obj)`.

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

## Exemple : `lint.rhai`

Un script de distribution minimal avec une règle maison. Il construit ses constats lui-même et n'a besoin
d'aucun import :

```rhai
// Règle maison : tout Deployment du groupe apps porte un label `team`.
fn lint(objs, ctx) {
    let findings = [];
    for obj in objs.all {
        if obj.apiVersion.starts_with("apps/") && obj.kind == "Deployment" {
            let labels = if "labels" in obj.metadata { obj.metadata.labels } else { #{} };
            if !("team" in labels) {
                findings.push(#{
                    rule: "team-label",
                    severity: "error",
                    object: #{
                        apiVersion: obj.apiVersion,
                        kind: obj.kind,
                        namespace: obj.metadata.namespace,
                        name: obj.metadata.name
                    },
                    path: "metadata.labels",
                    message: "le label `team` est obligatoire"
                });
            }
        }
    }
    findings
}
```

Lancement :

```bash
agent package test -p ./my-package --all --lint-dir ./distribution/lint
```

Un Deployment sans label `team` produit une ligne en échec `lint:team-label`. Un script qui lit `objs.all`
identifie un objet par son groupe d'API et son kind ensemble, jamais par son kind seul : un `Deployment`
d'un autre groupe n'est pas la charge de travail visée par la règle.

## Limites

- Les valeurs du budget d'opérations et leur configuration ne sont pas encore fixées.
- Un constat `warn` n'a pas de champ de sévérité dans le verdict : c'est une ligne réussie à préfixe, listée en `[WARN]` dans le rapport JUnit.
