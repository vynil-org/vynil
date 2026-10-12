# Architecture — Vynil

## Vue d'ensemble

Vynil est un gestionnaire de paquets pour Kubernetes. Son objectif est de fournir une
distribution intégrée de Kubernetes, à la manière de dpkg/rpm mais pour le cluster.
Contrairement à Helm, ArgoCD ou Flux, Vynil vise l'intégration par défaut plutôt que la
flexibilité maximale.

Le projet est un **workspace Rust** composé de trois crates :

```
vynil/
├── common/     — bibliothèque partagée : CRDs, types, moteurs de script
├── operator/   — contrôleur Kubernetes (binaire + lib)
└── agent/      — outil CLI (binaire + lib)
```

---

## Composants

### common (bibliothèque)

Contient tous les types partagés entre l'opérateur et l'agent :

- **CRDs Kubernetes** : définitions des quatre ressources personnalisées
- **Moteur Rhai** : intégration du langage de script (40+ fonctions exposées)
- **Moteur Handlebars** : rendu de templates (30+ helpers)
- **Handlers** : OCI, HTTP, YAML, mots de passe, semver, hachages
- **Macros** : génération de code répétitif pour les conditions de statut

### operator (contrôleur)

Binaire `operator` — serveur HTTP Actix sur le port 9000 + quatre contrôleurs kube-rs.

Responsabilités :
- Surveiller les CRDs (`JukeBox`, `TenantInstance`, `ServiceInstance`, `SystemInstance`)
- Mettre en cache les packages disponibles (depuis le statut des `JukeBox`)
- Pour chaque instance : sélectionner le bon package, vérifier les prérequis, créer le Job
- Exposer les métriques Prometheus (`GET /metrics`)

### agent (CLI)

Binaire `agent` — outil en ligne de commande lancé dans des Jobs Kubernetes.

Sous-commandes principales :
- `package {build,update,test,validate,unpack,lint}` — gestion du cycle de vie des packages OCI et linting statique
- `{system,service,tenant} {install,delete,reconfigure,backup,restore}` — opérations d'instance
- `crdgen` — génération des manifestes CRD
- `box`, `template`, `run` — utilitaires
- `box file-scan` — scan standalone vers fichiers (sans K8s)

### kubectl-vynil (plugin kubectl)

Binaire `kubectl-vynil` — plugin `kubectl` côté client (`kubectl vynil …`). Pilote l'opérateur
depuis le contexte kubeconfig de l'utilisateur : déclenche upgrades/scans, récupère diagnostics,
affiche les logs. Embarque un shim `kubectl_complete-vynil` pour la complétion shell dynamique (voir ci-dessous).

---

## Complétion shell

Le shim `kubectl_complete-vynil` implémente la complétion shell dynamique pour `kubectl vynil …` via le
protocole Cobra `__complete` (kubectl 1.26+) : kubectl invoque le shim, qui délègue au verbe caché
`kubectl-vynil __complete`. Le binaire `kubectl-vynil` embarque aussi une sous-commande
`completion <shell>` pour sourcer les scripts de complétion lors d'appels directs au binaire.

---

## Ressources Kubernetes personnalisées

Toutes dans le groupe `vynil.solidite.fr/v1`.

### JukeBox (cluster-scoped)

Source de packages Vynil. Contient une définition de source (liste OCI, projet Harbor, script,
URL HTTP ou bucket S3) et un planning de scan (cron). Le statut stocke la liste des packages
disponibles (waypoints d'upgrade).

```
spec:
  source:  List | Harbor | Script | Http | S3
  maturity: stable | beta | alpha
  pull_secret: <nom du secret imagePull>
  schedule: <expression cron>

status:
  packages: [VynilPackage]   ← cache des packages scannés
```

Types de source :
- `List` / `Harbor` / `Script` : scan direct des registres OCI
- `Http` : URL vers un cache de packages pré-calculés ; auth Basic ou Bearer via Secret
- `S3` : bucket S3/MinIO/OVH avec prefix optionnel ; credentials via Secret ou IAM role

### TenantInstance / ServiceInstance (namespaced)

Installation d'un package pour un tenant ou un service. Les deux types partagent la même
structure, avec backup/restore.

```
spec:
  jukebox, category, package
  init_from:
    secret_name, sub_path, snapshot
    version: <version exacte pour restauration>
  options: { clé: valeur }

status:
  tag:    <version actuellement installée>
  digest: <empreinte des options>
  conditions: [Ready, Installed, BeforeApplied, VitalApplied, ...]
```

### SystemInstance (namespaced)

Installation d'un package système (cluster-level). Pas de backup/restore, pas d'`initFrom`.

```
spec:
  jukebox, category, package
  options: { clé: valeur }

status:
  tag, digest
  conditions: [Ready, Installed, SystemApplied, ...]
```

---

## Format des packages

Un package Vynil est une **image OCI** avec des annotations de métadonnées :

| Annotation | Contenu |
|---|---|
| `fr.solidite.vynil.metadata` | JSON : nom, catégorie, type (tenant/system/service), features |
| `fr.solidite.vynil.requirements` | JSON : prérequis (CRDs, versions, ressources) |
| `fr.solidite.vynil.options` | JSON : paramètres configurables |
| `fr.solidite.vynil.recommandations` | JSON : recommandations (services, CRDs) |
| `fr.solidite.vynil.value_script` | Script Rhai pour valeurs dynamiques |

Le contenu de l'image contient les scripts Rhai du cycle de vie du package.

### Prérequis d'un package (`VynilPackageRequirement`)

- `MinimumPreviousVersion` — version minimale déjà installée pour pouvoir mettre à jour
- `VynilVersion` — version minimale du framework Vynil
- `ClusterVersion` — version minimale de Kubernetes
- `CustomResourceDefinition`, `SystemService`, `TenantService` — dépendances d'autres packages
- `Cpu`, `Memory`, `Disk`, `StorageCapability` — ressources nécessaires
- `Prefly` — script Rhai de vérification personnalisée

---

## Flux de réconciliation

### Scan du jukebox

```
JukeBox CRD
    → (cron) Job agent "scan.rhai"
        → sources OCI (List/Harbor/Script) : liste les tags OCI
        → sources Http/S3 : télécharge index.yaml + fichiers packages depuis le cache
        → filtre : semver valide + maturité + version Vynil compatible
        → filtre partiel si annotation force-scan=<category>[/<name>] présente
        → conserve les waypoints d'upgrade (1 version par "époque" de MinimumPreviousVersion)
        → met à jour JukeBox.status.packages
```

Les waypoints permettent une mise à jour progressive sans stocker toutes les versions.
Exemple : versions disponibles [4.0(min:3.0), 3.5(min:2.0), 3.0(min:2.0), 2.5, 1.5]
→ stocke [4.0, 3.5, 2.5, 1.5]

### Scan standalone (file-scan)

```
agent box file-scan
    → lit une spec JukeBox YAML locale (source + pull_secret fichier)
    → scanne les registres OCI sans connexion K8s
    → calcule les waypoints pour 3 niveaux de maturité et stocke l'union
    → produit <cache_dir>/index.yaml + <cache_dir>/<category>_<name>.yaml
    [upload vers HTTP/S3]
JukeBox source Http/S3
    → télécharge index.yaml + fichiers packages
    → applique filtre maturity + recalcule waypoints
    → met à jour JukeBox.status.packages (identique aux sources OCI)
```

### Réconciliation d'une instance

```
Instance CRD (TenantInstance / ServiceInstance / SystemInstance)
    → operator : do_reconcile<T>()
        1. current_version = status.tag (vide si premier install)
        2. sélection du package dans le cache jukebox :
           - nom + catégorie + type correspondants
           - is_min_version_ok(current_version) — chaîne d'upgrade respectée
           - is_vynil_version_ok() — framework compatible
        3. vérification des prérequis (CRDs, services, resources...)
        4. construction des recommandations (listes CRDs/services optionnels)
        5. exécution du value_script Rhai (si présent)
        6. [si initFrom.version et premier install] vérification tag OCI directe
        7. rendu du Job Kubernetes via template Handlebars (package.yaml.hbs)
        8. création/mise à jour du Job (Server-Side Apply)
        → requeue toutes les 15 minutes
```

### Annotations de contrôle sur les instances

| Annotation | Valeur | Effet |
|---|---|---|
| `vynil.solidite.fr/suspend` | `"true"` | Suspend la réconciliation jusqu'à suppression de l'annotation. Le controller requeue normalement (15 min) mais ne fait rien. |
| `vynil.solidite.fr/force-reinstall` | présente | Force la réinstallation : supprime le Job existant avant de le recréer, puis retire l'annotation automatiquement. |

### Annotations de contrôle sur les JukeBox

| Annotation | Valeur | Comportement |
|---|---|---|
| `vynil.solidite.fr/force-scan` | `"true"` (ou présente sans valeur) | Scan complet de tous les packages |
| `vynil.solidite.fr/force-scan` | `"<category>"` | Scan partiel de toute la catégorie |
| `vynil.solidite.fr/force-scan` | `"<category>/<name>"` | Scan partiel d'un seul package |

### Suppression d'une instance (finalizer)

```
Instance CRD (suppression)
    → do_cleanup<T>()
        → même sélection de package
        → Job avec action "delete"
        → attente de completion
        → retrait du finalizer
```

---

## Pattern générique : trait InstanceKind

Les trois types d'instance partagent un seul algorithme de réconciliation via le trait
`InstanceKind` dans `operator/src/instance_common.rs`.

Méthodes clés du trait :

| Méthode | Rôle |
|---|---|
| `type_name()`, `package_type()` | Constantes de type |
| `spec_jukebox()`, `spec_category()`, `spec_package()` | Accesseurs spec |
| `current_tag()` | Version installée (depuis `status.tag`) |
| `init_from_version()` | Version de restauration (`initFrom.version`), default `None` |
| `check_requirements()` | Vérifie les prérequis du package |
| `build_recommendations()` | Construit les listes de recommandations |
| `set_rhai_instance()` | Injecte l'instance dans le scope Rhai |
| `set_missing_box()`, `set_missing_package()`, ... | Mises à jour de statut d'erreur |

---

## Stratégie YAML

| Usage | Bibliothèque | Ordre des clés |
|---|---|---|
| Tout le code Rust (sérialisation/désérialisation) | `serde_yaml` | Alphabétique |
| `yaml_decode_ordered` / `yaml_encode_ordered` (Rhai) | `rust-yaml` | Préservé |

Le type `YamlError(String)` dans `common/src/lib.rs` encapsule les deux bibliothèques.
`rust-yaml` est utilisé uniquement dans `update.rhai` pour ne pas réordonner les clés de
`package.yaml`.

---

## Scripts Rhai

Les scripts Rhai sont embarqués dans les images OCI des packages et exécutés par l'agent.
Le moteur Rhai est configuré dans `common/src/rhaihandler.rs`.

Bibliothèque de scripts réutilisables (`agent/scripts/lib/`) :
- `secret_dockerconfigjson.rhai` — lecture des secrets imagePull
- `scan_harbor.rhai` — listing des dépôts Harbor

Scripts de l'agent (`agent/scripts/`) :
- `boxes/scan.rhai` — scan du jukebox
- `packages/{build,update,test,validate}.rhai` — cycle de vie des packages
- `service/`, `tenant/`, `system/` — hooks d'installation, suppression, backup, restauration

---

## Templates Handlebars

Répertoire : `operator/templates/`

| Template | Usage |
|---|---|
| `package.yaml.hbs` | Job d'installation/suppression d'une instance |
| `cronscan.yaml.hbs` | CronJob de scan d'un JukeBox |
| `scan.yaml.hbs` | Job de scan manuel d'un JukeBox |

Variables systématiquement disponibles dans le contexte : `tag`, `image`, `registry`,
`namespace`, `name`, `job_name`, `package_type`, `package_action`, `digest`, `ctrl_values`.

---

## Métriques

L'opérateur expose des métriques Prometheus sur `GET /metrics` (format OpenMetrics).

Quatre registres séparés (un par type de ressource) exposent :
- Durée des réconciliations (histogramme)
- Compteurs de succès/échec
- Jauge des réconciliations en cours
- Horodatage du dernier événement

---

## Tests de régression Rhai

La suite de tests dans `agent/tests/rhai_*.rs` exécute les scripts Rhai internes de l'agent avec un moteur Rhai réel et des mocks K8s/HTTP. Elle est structurée en deux niveaux :

- **`rhai_lib.rs`** : tests unitaires des scripts `agent/scripts/lib/` (fonctions isolées, assertions sur valeurs retournées)
  - Couvre les patterns susceptibles de régresser lors des mises à jour Rhai (`.filter()`, `.replace()`, `.reduce()`, closures)
  - Environ 20 tests unitaires couvrant storage_class, wait, install_from_dir, gen_package, backup_context, resolv_service

- **`rhai_lifecycle.rs`** : tests d'intégration des scripts de cycle de vie service/install et service/delete (flow complet avec mocks K8s)
  - Exécute les flows end-to-end : context → install / context → delete
  - Valide que tous les scripts lib/ s'assemblent correctement

Ces tests servent de filet de régression lors des mises à jour de la version Rhai, en capturant les changements de sémantique des fonctions de manipulation de strings et collections.

---

## Variables d'environnement (operator)

| Variable | Défaut | Rôle |
|---|---|---|
| `CONTROLLER_BASE_DIR` | `./operator` | Répertoire des templates Handlebars |
| `VYNIL_NAMESPACE` | `vynil-system` | Namespace système de Vynil |
| `AGENT_IMAGE` | `ghcr.io/vynil-org/vynil-agent:0.7.10` | Image de l'agent pour les Jobs |
| `AGENT_ACCOUNT` | `vynil-agent` | ServiceAccount des Jobs |
| `AGENT_LOG_LEVEL` | `info` | Niveau de log |
| `TENANT_LABEL` | `vynil.solidite.fr/tenant` | Clé du label tenant |
| `SCAN_PACKAGE` | (absent) | Filtre partiel pour `box scan` et `box file-scan` |

---

## Dépendances principales

| Crate | Version | Rôle |
|---|---|---|
| `kube` | 3.1.0 | Client Kubernetes + contrôleurs |
| `k8s-openapi` | 0.27.1 | Types Kubernetes |
| `rhai` | ~1.20 | Moteur de script |
| `handlebars` | ~6 | Rendu de templates |
| `oci-client` | ~0.12 | Registre OCI |
| `serde_yaml` | 0.9 | Sérialisation YAML (tri alphabétique) |
| `rust-yaml` | git `sebt3/rust-yaml` | Sérialisation YAML à ordre préservé |
| `tokio` | 1.48 | Runtime async |
| `actix-web` | 4.12 | Serveur HTTP métriques |
| `prometheus-client` | ~0.22 | Métriques Prometheus |

> Version du workspace au moment de la rédaction : **0.7.10** (édition Rust 2024, licence BSD-3-Clause).
