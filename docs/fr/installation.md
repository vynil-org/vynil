# Installation

## Prérequis

- Un cluster Kubernetes et `kubectl` configuré (droits cluster-admin pour l'installation
  initiale — voir la note de sécurité plus bas).
- Un accès réseau au registre OCI hébergeant les paquets (par défaut `ghcr.io/vynil-org/vynil`).

## Installation de l'opérateur

```bash
kubectl create ns vynil-system
kubectl apply -k github.com/vynil-org/vynil//deploy
```

Le kustomize `deploy/` installe :

- les **CRD** (`deploy/crd/`) : `JukeBox`, `SystemInstance`, `ServiceInstance`,
  `TenantInstance` ;
- le **bootstrap** (`deploy/bootstrap/`) : un ServiceAccount `vynil-bootstrap`, une
  `JukeBox` nommée `vynil` pointant vers `ghcr.io/vynil-org/vynil`, une `SystemInstance`
  `vynil`, et un Job d'amorçage qui scanne la JukeBox puis installe le paquet `core/vynil`
  (qui déploie à son tour l'opérateur lui-même).

Vynil s'installe donc *via Vynil* : le bootstrap pose juste assez de matière pour que
l'opérateur prenne le relais et se gère ensuite comme n'importe quel paquet système.

## Vérifier l'installation

```bash
# l'opérateur tourne
kubectl -n vynil-system get pods

# le catalogue de la jukebox de référence est peuplé
kubectl get jukebox vynil -o jsonpath='{.status.packages[*].metadata.name}'

# la SystemInstance vynil est Ready
kubectl -n vynil-system get systeminstances
```

## Ajouter une source de paquets

Créez une `JukeBox` pointant vers votre registre :

```yaml
apiVersion: vynil.solidite.fr/v1
kind: JukeBox
metadata:
  name: home-alpha
spec:
  source:
    list:
    - "registry.example.com/my-org/vynil"
  maturity: stable
  schedule: "0 3 * * *"        # rescan quotidien à 3h
  # pull_secret: my-pull-secret # si registre privé
```

Forcer un scan immédiat sans attendre le cron :

```bash
kubectl annotate jukebox home-alpha vynil.solidite.fr/force-scan=true --overwrite
```

Voir [Sources de JukeBox](jukebox/sources.md) pour les sources Harbor, GitLab, HTTP et S3.

## Installer un paquet

```yaml
apiVersion: vynil.solidite.fr/v1
kind: TenantInstance
metadata:
  name: my-ollama
  namespace: my-namespace
spec:
  jukebox: home-alpha
  category: think
  package: ollama
  options:
    use_rocm: true
```

Suivre l'avancement :

```bash
kubectl -n my-namespace get tenantinstances
kubectl -n my-namespace describe tenantinstance my-ollama   # conditions détaillées
kubectl -n vynil-system get jobs                            # job d'install de l'agent
```

## Désinstaller

```bash
kubectl -n my-namespace delete tenantinstance my-ollama
```

La suppression est gérée par un finalizer : l'opérateur lance un Job de `delete` qui
nettoie les enfants enregistrés dans le `status`, puis retire le finalizer. Si une
désinstallation reste bloquée, voir [Dépannage](operations/troubleshooting.md).

## Note de sécurité importante

Par défaut, l'agent Vynil s'exécute avec des droits **cluster-admin** et exécute le code
(Rhai) embarqué dans les paquets. **N'installez que des paquets issus de JukeBox de
confiance.** Lisez [Sécurité & modèle de menace](operations/security.md) avant tout
déploiement en production.

## Variables d'environnement de l'opérateur

Les principales variables sont décrites dans la [Référence](operations/reference.md)
(`VYNIL_NAMESPACE`, `AGENT_IMAGE`, `AGENT_ACCOUNT`, `TENANT_LABEL`, etc.).
