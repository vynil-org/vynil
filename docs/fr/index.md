# Vynil — gestionnaire de paquets pour Kubernetes

> Vynil est à Kubernetes ce que `dpkg`/`rpm` sont à une distribution Linux : un
> gestionnaire de paquets dont le but est de produire une **distribution Kubernetes
> intégrée**, et non d'offrir une flexibilité de déploiement maximale.

## En une phrase

Vous décrivez une **source de paquets** (`JukeBox`) et des **installations**
(`SystemInstance`, `ServiceInstance`, `TenantInstance`) sous forme de ressources
Kubernetes ; l'opérateur Vynil réconcilie ces objets en lançant un **agent** dans des
Jobs qui déploient, mettent à jour, sauvegardent et désinstallent les applications.

```mermaid
flowchart LR
    subgraph Catalogue
      JB[JukeBox] -->|scan cron| PK[(status.packages)]
    end
    subgraph Installations
      SI[SystemInstance]
      VI[ServiceInstance]
      TI[TenantInstance]
    end
    OP[Opérateur Vynil] -->|sélection package| PK
    SI --> OP
    VI --> OP
    TI --> OP
    OP -->|crée un Job| AG[Agent k8s Job]
    AG -->|Rhai + Handlebars| K8S[(Objets Kubernetes)]
```

## Objectif primaire et positionnement

Contrairement à Helm, Kustomize, ArgoCD ou Flux — qui donnent toute latitude pour
installer comme bon vous semble — Vynil vise **l'intégration par défaut**. La
personnalisation y est volontairement réduite, mais tout s'intègre nativement avec le
reste de la distribution. Vynil se distingue aussi d'OLM (OpenShift) : OLM n'installe que
des opérateurs, alors que Vynil est un opérateur d'installation *générique*. Il peut
installer une application simple (phpMyAdmin), une application avec état et sauvegarde
(une base de données) ou un composant cluster unique (kube-virt) sans exiger un opérateur
dédié par application.

La valeur ajoutée est l'**opiniâtreté** : un paquet Vynil fige les décisions d'intégration
(ressources, stockage, réseau, sécurité, dépendances) qu'un chart générique laisse à la
charge de chaque utilisateur — voir
[Construire une distribution](distribution.md). Le format paquet = image OCI apporte le
reste : immutabilité, auditabilité, air-gap —
voir [Le paquet OCI](packages/portability.md).

## Cas d'usage

Vynil est un cadre générique : le même moteur couvre des usages très différents.

| Cas d'usage | En quelques mots |
|---|---|
| **Distribution communautaire** | Reproduire l'écosystème d'une distribution Linux (à la Debian) dans Kubernetes : un catalogue intégré, maintenu par une communauté. |
| **Distribution d'entreprise** | Une plateforme interne intégrée à l'écosystème existant (SSO, stockage, réseau, conformité) ; les équipes consomment des paquets pré-intégrés. |
| **Orchestration SaaS** | Le client commande son tenant et ses options dans l'interface du produit ; le produit crée des `TenantInstance` et Vynil déploie et maintient. Le produit lui-même peut être distribué comme paquet Vynil. |
| **Orchestration d'infrastructure cloud** | La phase OpenTofu des paquets pilote des ressources hors cluster (DNS, buckets, bases managées…) dans le même cycle de vie. |
| **Platform-as-a-Service depuis Kubernetes** | Définir une plateforme self-service (dans l'esprit de Crossplane, sans sa prolifération de CRDs) : les capacités sont des paquets, la surface utilisateur des instances. |
| **Packaging amont** | Un projet open-source publie directement sa propre box — le « paquet officiel » du projet, signé par l'amont, consommé via une simple JukeBox supplémentaire. |

## Le modèle mental en trois objets

| Objet | Portée | Rôle |
|---|---|---|
| **JukeBox** | cluster | Source de paquets. Scanne périodiquement un registre OCI (ou un cache HTTP/S3) et publie la liste des paquets disponibles dans son `status`. |
| **SystemInstance** | namespace | Installe un paquet *système* (composant cluster, sans sauvegarde). |
| **ServiceInstance** | namespace | Installe un paquet *service* (application partagée, avec CRDs propres et sauvegarde). |
| **TenantInstance** | namespace | Installe un paquet *tenant* (application cantonnée à un tenant, avec sauvegarde/restauration). |

Un **paquet** est une **image OCI** : ses métadonnées sont portées par des annotations
OCI, et son contenu embarque des templates Handlebars et des scripts Rhai décrivant son
cycle de vie.

## Par où commencer

- **Découvrir le modèle** → [Concepts](concepts.md)
- **Installer Vynil** → [Installation](installation.md)
- **Comprendre le moteur** → [Architecture](architecture.md) et [Réconciliation](reconciliation.md)
- **Construire une distribution** → [Distribution](distribution.md), [Le paquet OCI](packages/portability.md)
- **Écrire un paquet** → [Format d'un paquet](packages/format.md), [Cycle de vie](packages/lifecycle.md), [Génération](gen-package.md)
- **Publier des paquets** → [Sources de JukeBox](jukebox/sources.md), [Build & signature](build-signing.md), [Maintenance du registre](jukebox/registry-maintenance.md)
- **Outiller** → [Référence CLI de l'agent](cli.md), [kubectl-vynil](tooling/kubectl-vynil.md), [Lint](tooling/lint.md), [Tests de paquet](tooling/test.md)
- **Exploiter** → [Sécurité & modèle de menace](operations/security.md), [Dépannage](operations/troubleshooting.md), [Référence](operations/reference.md)

## Note pour les assistants (LLM)

Un index lisible par machine est disponible à la racine du dépôt :
[`llms.txt`](../llms.txt). Il liste les pages de cette documentation avec une courte
description, au format [llmstxt.org](https://llmstxt.org). Toutes les pages sont du
Markdown brut, directement consommable.

## Licence, état et crédits

BSD-3-Clause. Projet en développement actif (workspace en version `0.7.10`). Dépôt :
`vynil-org/vynil`.

Documentation rédigée et maintenue par les mainteneurs, à partir du code et du
retour d'expérience d'exploitation de distributions Vynil en production.
