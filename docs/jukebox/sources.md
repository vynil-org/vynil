# JukeBox Sources

A `JukeBox` declares **one** source variant in `spec.source`. The scan
([Reconciliation](../reconciliation.md)) reads it to produce the
`status.packages` catalog.

## List — list of OCI repositories

The simplest form: a list of OCI images to scan directly.

```yaml
spec:
  source:
    list:
    - "ghcr.io/vynil-org/vynil"
    - "registry.example.com/org/another-set"
  maturity: stable
  schedule: "0 3 * * *"  # daily rescan at 3am
  pull_secret: my-pull-secret   # if private registry
```

The scan lists the tags of each repository, keeps only valid semver tags, applies
the maturity filter, and computes upgrade waypoints.

## Harbor — Harbor project

Scans all repositories in a Harbor project (API host and OCI host are the same).

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

GitLab separates the API host (`url`) from the OCI registry host (`registry`). Full details and
token strategies (PAT, `CI_JOB_TOKEN`, deploy token) in the dedicated guide:
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

## Script — Rhai-driven scan

For non-standard registries, a Rhai script provides the list of repositories to scan. Useful
when enumerating images requires specific API logic.

## Http — pre-computed package cache

Instead of scanning a registry, the JukeBox downloads a pre-computed index and package files
(produced by `agent box file-scan`). Ideal for decoupling the scan (expensive, off-cluster)
from consumption.

```yaml
spec:
  source:
    http:
      url: "https://cache.example.com/vynil/"
      # auth Basic or Bearer via Secret
  maturity: stable
  schedule: "*/30 * * * *"
```

The scan fetches `index.yaml` then the `<category>_<name>.yaml` files, applies the
maturity filter and recomputes waypoints — the result is identical to a direct OCI scan.

## S3 — bucket S3/MinIO/OVH

Same principle as `http`, but the cache is stored in an S3 bucket.

```yaml
spec:
  source:
    s3:
      bucket: "vynil-cache"
      endpoint: "https://s3.example.com"   # MinIO/OVH compatible
      prefix: "packages/"                   # optional
      # credentials via Secret or IAM role
  maturity: stable
  schedule: "*/30 * * * *"
```

## Maturity and waypoints

Regardless of source, the scan applies the chosen `maturity` and keeps only one
**waypoint per epoch** of `MinimumPreviousVersion`, ensuring a consistent upgrade
chain without storing every version. The standalone scan (`file-scan`) computes the union
of waypoints for all three maturity levels, and the consuming JukeBox (http/s3)
then applies its own maturity filter.

## Forcing a scan

```bash
# immediate full scan
kubectl annotate jukebox <name> vynil.solidite.fr/force-scan=true --overwrite
# partial scan of a category or package
kubectl annotate jukebox <name> vynil.solidite.fr/force-scan="database" --overwrite
kubectl annotate jukebox <name> vynil.solidite.fr/force-scan="database/postgresql" --overwrite
```
