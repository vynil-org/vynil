# Sources de JukeBox

Une `JukeBox` déclare **une** variante de source dans `spec.source`. Le scan
([Réconciliation](../reconciliation.md)) la lit pour produire le catalogue
`status.packages`.

## List — liste de dépôts OCI

La forme la plus simple : une liste d'images OCI à scanner directement.

```yaml
spec:
  source:
    list:
    - "ghcr.io/vynil-org/vynil"
    - "registry.example.com/org/another-set"
  maturity: stable
  schedule: "0 3 * * *"
  pull_secret: my-pull-secret   # optionnel
```

Le scan liste les tags de chaque dépôt, ne garde que les tags semver valides, applique le
filtre de maturité et calcule les waypoints d'upgrade.

## Harbor — projet Harbor

Scanne tous les dépôts d'un projet Harbor (l'hôte API et l'hôte OCI sont identiques).

```yaml
spec:
  source:
    harbor:
      url: "https://harbor.example.com"
      project: "vynil"
  maturity: beta
  schedule: "0 */6 * * *"
  pull_secret: harbor-pull-secret
```

## GitLab — GitLab Container Registry

GitLab dissocie l'hôte API (`url`) de l'hôte registre OCI (`registry`). Détails complets et
stratégies de tokens (PAT, `CI_JOB_TOKEN`, deploy token) dans le guide dédié :
[GitLab Container Registry](gitlab-registry.md).

```yaml
spec:
  source:
    gitlab:
      url: "https://gitlab.com"          # API REST v4
      registry: "registry.gitlab.com"    # push/pull OCI
      project: "my-group/my-project"
  maturity: stable
  schedule: "0 3 * * *"
```

## Script — scan piloté par Rhai

Pour les registres non standard, un script Rhai fournit la liste des dépôts à scanner. Utile
quand l'énumération des images nécessite une logique d'API spécifique.

## Http — cache de paquets pré-calculé

Au lieu de scanner un registre, la JukeBox télécharge un index et des fichiers de paquets
déjà calculés (produits par `agent box file-scan`). Idéal pour découpler le scan
(coûteux, hors cluster) de la consommation.

```yaml
spec:
  source:
    http:
      url: "https://cache.example.com/vynil/"
      # auth Basic ou Bearer via Secret
  maturity: stable
  schedule: "*/30 * * * *"
```

Le scan récupère `index.yaml` puis les `<category>_<name>.yaml`, applique le filtre de
maturité et recalcule les waypoints — le résultat est identique à un scan OCI direct.

## S3 — bucket S3/MinIO/OVH

Même principe que `http`, mais le cache est stocké dans un bucket S3.

```yaml
spec:
  source:
    s3:
      bucket: "vynil-cache"
      endpoint: "https://s3.example.com"   # MinIO/OVH compatible
      prefix: "packages/"                   # optionnel
      # credentials via Secret ou rôle IAM
  maturity: stable
  schedule: "*/30 * * * *"
```

## Maturité et waypoints

Quelle que soit la source, le scan applique la `maturity` choisie et ne conserve qu'un
**waypoint par époque** de `MinimumPreviousVersion`, garantissant une chaîne de mise à jour
cohérente sans stocker toutes les versions. Le scan standalone (`file-scan`) calcule l'union
des waypoints pour les trois niveaux de maturité, et c'est la JukeBox consommatrice (http/s3)
qui applique ensuite son propre filtre de maturité.

## Forcer un scan

```bash
# scan complet immédiat
kubectl annotate jukebox <name> vynil.solidite.fr/force-scan=true --overwrite
# scan partiel d'une catégorie ou d'un paquet
kubectl annotate jukebox <name> vynil.solidite.fr/force-scan="database" --overwrite
kubectl annotate jukebox <name> vynil.solidite.fr/force-scan="database/postgresql" --overwrite
```
