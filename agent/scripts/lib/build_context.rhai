
import "storage_class_enrich" as sce;
fn appslug(pkg, inst) {
    if pkg == inst {
        inst
    } else if `${inst}-${pkg}`.len() > 28 {
        let res = `${inst}-${pkg}`;
        res.truncate(28);
        res
    } else {
        `${inst}-${pkg}`
    }
}
fn get_default(def) {
    let r = #{};
    if def != () && type_of(def) == "map" {
        for k in def.keys() {
            r[k] = def[k]["default"];
        }
    }
    r
}

fn get_values(options, defaults) {
    if type_of(options) == "()" {
        defaults
    } else if type_of(defaults) == "map" && type_of(options) == "map" {
        let r = #{};
        for k in defaults.keys() {
            r[k] = get_values(options[k],defaults[k]);
        }
        r
    } else {
        options
    }
}

fn get_default_context(args) {
    if args.context_name == "HA" {
        #{
            nodes: ["master01","master02","master03","worker01","worker02","worker03","worker04","worker05"],
            crds: [
                "alertmanagerconfigs.monitoring.coreos.com",
                "alertmanagers.monitoring.coreos.com",
                "alerts.notification.toolkit.fluxcd.io",
                "backups.postgresql.cnpg.io",
                "basicauths.secretgenerator.mittwald.de",
                "bindings.rabbitmq.com",
                "bucketaccessclasses.objectstorage.k8s.io",
                "bucketaccesses.objectstorage.k8s.io",
                "bucketclaims.objectstorage.k8s.io",
                "bucketclasses.objectstorage.k8s.io",
                "buckets.objectstorage.k8s.io",
                "buckets.source.toolkit.fluxcd.io",
                "cdiconfigs.cdi.kubevirt.io",
                "cdis.cdi.kubevirt.io",
                "cephblockpoolradosnamespaces.ceph.rook.io",
                "cephblockpools.ceph.rook.io",
                "cephbucketnotifications.ceph.rook.io",
                "cephbuckettopics.ceph.rook.io",
                "cephclients.ceph.rook.io",
                "cephclusters.ceph.rook.io",
                "cephcosidrivers.ceph.rook.io",
                "cephfilesystemmirrors.ceph.rook.io",
                "cephfilesystems.ceph.rook.io",
                "cephfilesystemsubvolumegroups.ceph.rook.io",
                "cephnfses.ceph.rook.io",
                "cephobjectrealms.ceph.rook.io",
                "cephobjectstores.ceph.rook.io",
                "cephobjectstoreusers.ceph.rook.io",
                "cephobjectzonegroups.ceph.rook.io",
                "cephobjectzones.ceph.rook.io",
                "cephrbdmirrors.ceph.rook.io",
                "certificaterequests.cert-manager.io",
                "certificates.cert-manager.io",
                "challenges.acme.cert-manager.io",
                "ciliumcidrgroups.cilium.io",
                "ciliumclusterwidenetworkpolicies.cilium.io",
                "ciliumegressgatewaypolicies.cilium.io",
                "ciliumendpoints.cilium.io",
                "ciliumexternalworkloads.cilium.io",
                "ciliumidentities.cilium.io",
                "ciliuml2announcementpolicies.cilium.io",
                "ciliumloadbalancerippools.cilium.io",
                "ciliumnetworkpolicies.cilium.io",
                "ciliumnodeconfigs.cilium.io",
                "ciliumnodes.cilium.io",
                "ciliumpodippools.cilium.io",
                "clusterimagecatalogs.postgresql.cnpg.io",
                "clusterissuers.cert-manager.io",
                "clusters.postgresql.cnpg.io",
                "databases.postgresql.cnpg.io",
                "dataimportcrons.cdi.kubevirt.io",
                "datasources.cdi.kubevirt.io",
                "datavolumes.cdi.kubevirt.io",
                "exchanges.rabbitmq.com",
                "federations.rabbitmq.com",
                "gitrepositories.source.toolkit.fluxcd.io",
                "grafanaalertrulegroups.grafana.integreatly.org",
                "grafanacontactpoints.grafana.integreatly.org",
                "grafanadashboards.grafana.integreatly.org",
                "grafanadatasources.grafana.integreatly.org",
                "grafanafolders.grafana.integreatly.org",
                "grafanalibrarypanels.grafana.integreatly.org",
                "grafanamutetimings.grafana.integreatly.org",
                "grafananotificationpolicies.grafana.integreatly.org",
                "grafananotificationpolicyroutes.grafana.integreatly.org",
                "grafananotificationtemplates.grafana.integreatly.org",
                "grafanas.grafana.integreatly.org",
                "helmcharts.source.toolkit.fluxcd.io",
                "helmreleases.helm.toolkit.fluxcd.io",
                "helmrepositories.source.toolkit.fluxcd.io",
                "imagecatalogs.postgresql.cnpg.io",
                "imagepolicies.image.toolkit.fluxcd.io",
                "imagerepositories.image.toolkit.fluxcd.io",
                "imageupdateautomations.image.toolkit.fluxcd.io",
                "ingressroutes.traefik.io",
                "ingressroutetcps.traefik.io",
                "ingressrouteudps.traefik.io",
                "ippools.whereabouts.cni.cncf.io",
                "issuers.cert-manager.io",
                "jukeboxes.vynil.solidite.fr",
                "kubeauthenticationconfigurations.kacp.solidite.fr",
                "kubevirts.kubevirt.io",
                "kustomizations.kustomize.toolkit.fluxcd.io",
                "middlewares.traefik.io",
                "middlewaretcps.traefik.io",
                "migrationpolicies.migrations.kubevirt.io",
                "network-attachment-definitions.k8s.cni.cncf.io",
                "objectbucketclaims.objectbucket.io",
                "objectbuckets.objectbucket.io",
                "objecttransfers.cdi.kubevirt.io",
                "ocirepositories.source.toolkit.fluxcd.io",
                "operatorpolicies.rabbitmq.com",
                "orders.acme.cert-manager.io",
                "overlappingrangeipreservations.whereabouts.cni.cncf.io",
                "permissions.rabbitmq.com",
                "podlogs.monitoring.grafana.com",
                "podmonitors.monitoring.coreos.com",
                "policies.rabbitmq.com",
                "poolers.postgresql.cnpg.io",
                "probes.monitoring.coreos.com",
                "prometheusagents.monitoring.coreos.com",
                "prometheuses.monitoring.coreos.com",
                "prometheusrules.monitoring.coreos.com",
                "providers.notification.toolkit.fluxcd.io",
                "publications.postgresql.cnpg.io",
                "queues.rabbitmq.com",
                "rabbitmqclusters.rabbitmq.com",
                "receivers.notification.toolkit.fluxcd.io",
                "redis.redis.redis.opstreelabs.in",
                "redisclusters.redis.redis.opstreelabs.in",
                "redisreplications.redis.redis.opstreelabs.in",
                "redissentinels.redis.redis.opstreelabs.in",
                "restendpoints.kuberest.solidite.fr",
                "scheduledbackups.postgresql.cnpg.io",
                "scheduledresources.cloud.namecheap.com",
                "schemareplications.rabbitmq.com",
                "scrapeconfigs.monitoring.coreos.com",
                "serverstransports.traefik.io",
                "serverstransporttcps.traefik.io",
                "servicemonitors.monitoring.coreos.com",
                "shovels.rabbitmq.com",
                "sshkeypairs.secretgenerator.mittwald.de",
                "storageprofiles.cdi.kubevirt.io",
                "stringsecrets.secretgenerator.mittwald.de",
                "subscriptions.postgresql.cnpg.io",
                "superstreams.rabbitmq.com",
                "systeminstances.vynil.solidite.fr",
                "tenantinstances.vynil.solidite.fr",
                "thanosrulers.monitoring.coreos.com",
                "tlsoptions.traefik.io",
                "tlsstores.traefik.io",
                "topicpermissions.rabbitmq.com",
                "traefikservices.traefik.io",
                "users.rabbitmq.com",
                "vhosts.rabbitmq.com",
                "virtualmachineclones.clone.kubevirt.io",
                "virtualmachineclusterinstancetypes.instancetype.kubevirt.io",
                "virtualmachineclusterpreferences.instancetype.kubevirt.io",
                "virtualmachineexports.export.kubevirt.io",
                "virtualmachineinstancemigrations.kubevirt.io",
                "virtualmachineinstancepresets.kubevirt.io",
                "virtualmachineinstancereplicasets.kubevirt.io",
                "virtualmachineinstances.kubevirt.io",
                "virtualmachineinstancetypes.instancetype.kubevirt.io",
                "virtualmachinepools.pool.kubevirt.io",
                "virtualmachinepreferences.instancetype.kubevirt.io",
                "virtualmachinerestores.snapshot.kubevirt.io",
                "virtualmachines.kubevirt.io",
                "virtualmachinesnapshotcontents.snapshot.kubevirt.io",
                "virtualmachinesnapshots.snapshot.kubevirt.io",
                "volumeclonesources.cdi.kubevirt.io",
                "volumeimportsources.cdi.kubevirt.io",
                "volumesnapshotclasses.snapshot.storage.k8s.io",
                "volumesnapshotcontents.snapshot.storage.k8s.io",
                "volumesnapshots.snapshot.storage.k8s.io",
                "volumeuploadsources.cdi.kubevirt.io",
            ],
            scs: [#{
                is_default: true,
                name: "rook-cephfs",
                provisioner: "storage.cephfs.csi.ceph.com"
            },#{
                is_default: false,
                name: "rook-ceph-block",
                provisioner: "storage.rbd.csi.ceph.com",
            },#{
                "is_default": false,
                "name": "local-path",
                "provisioner": "rancher.io/local-path"
            }]
        }
    } else {
        #{
            nodes: ["single"],
            crds: [],
            scs: [#{
                "is_default": true,
                "name": "local-path",
                "provisioner": "rancher.io/local-path"
            }]
        }
    }
}

fn template(instance, args) {
    let pkg = core::yaml_decode(host::file_read(`${args.package_dir}/package.yaml`));
    let defaults = get_default_context(args);
    let scs = sce::classes_enrich(defaults.scs);
    let current = "";
    let controller = #{};
    try {
        controller = core::json_decode(args.controller_values);
    } catch (e) {
        switch type_of(e) {
            "string" => host::log_warn(e),
            _ => host::log_warn(core::json_encode(e))
        }
    }
    let cluster_config = if host::is_file(`${args.config_dir}/agent.yaml`) {
        core::yaml_decode(host::file_read(`${args.config_dir}/agent.yaml`))
    } else {#{}};
    cluster_config["storage_classes"] = scs;
    cluster_config["crds"] = defaults.crds;
    cluster_config["services"] = [];
    if ! ("prefered_storage" in cluster_config.keys()) {
        cluster_config["prefered_storage"] = sce::get_prefered_sc(scs);
    }
    if ! ("ha" in cluster_config.keys()) {
        let nodes = defaults.nodes;
        cluster_config["ha"] = nodes.len() > 1;
    }
    cluster_config["vynil_namespace"] = args.vynil_namespace;
    let defaults = get_default(pkg.options);

    #{
        cluster: cluster_config,
        controller: controller,
        instance: #{
            name: instance.metadata.name,
            namespace: instance.metadata.namespace,
            "package": #{
                name: pkg.metadata.name,
                jukebox: "box",
                category: pkg.metadata.category,
                app_version: pkg.metadata.app_version,
                options: pkg.options,
            },
            labels: #{
                "app.kubernetes.io/name": pkg.metadata.name,
                "app.kubernetes.io/instance": instance.metadata.name,
                "app.kubernetes.io/managed-by": "vynil",
                "vynil.solidite.fr/owner-namespace": instance.metadata.namespace,
                "vynil.solidite.fr/owner-category": pkg.metadata.category,
                "vynil.solidite.fr/owner-type": "system",
            },
            selector: #{
                "app.kubernetes.io/name": pkg.metadata.name,
                "app.kubernetes.io/instance": instance.metadata.name,
            },
            images: pkg.images,
            resources: pkg.resources,
            current: current,
            requested: args.tag,
            appslug: appslug(pkg.metadata.name, instance.metadata.name)
        },
        values: get_values(#{}, defaults),
        defaults: defaults,
        package_dir: args.package_dir,
        config_dir: args.config_dir,
    }
}

fn run(instance, args) {
    let pkg = read_package_yaml(`${args.package_dir}/package.yaml`);
    let scs = sce::classes_enrich(k8s_resource("StorageClass").list().items.map(|s| #{ name: s.metadata.name, provisioner: s.provisioner, is_default: s.metadata.annotations != () && s.metadata.annotations["storageclass.kubernetes.io/is-default-class"]=="true" }));
    let current = "";
    let controller = #{};
    try {
        controller = core::json_decode(args.controller_values);
    } catch (e) {
        switch type_of(e) {
            "string" => host::log_warn(e),
            _ => host::log_warn(core::json_encode(e))
        }
    }
    if instance.status != () && instance.status.tag != () {
        current = instance.status.tag;
    }
    let cluster_config = if host::is_file(`${args.config_dir}/agent.yaml`) {
        core::yaml_decode(host::file_read(`${args.config_dir}/agent.yaml`))
    } else {#{}};
    cluster_config["storage_classes"] = scs;
    let crds = k8s_resource("CustomResourceDefinition").list_meta().items;
    cluster_config["crds"] = crds.map(|c| c.metadata.name);
    if ! ("prefered_storage" in cluster_config.keys()) {
        cluster_config["prefered_storage"] = sce::get_prefered_sc(scs);
    }
    if ! ("ha" in cluster_config.keys()) {
        let nodes = k8s_resource("Node").list_meta().items.map(|c| c.metadata.name);
        cluster_config["ha"] = nodes.len() > 1;
    }
    cluster_config["vynil_namespace"] = args.vynil_namespace;
    let defaults = get_default(pkg.options);
    cluster_config["services"] = [];
    try {
        let svcs = k8s_resource("ServiceInstance").list().items;
        let lst = [];
        if type_of(instance) == "TenantInstance" {
            let tenant = instance.get_tenant_name();
            for svc in svcs {
                if type_of(svc.status) == "map" && type_of(svc.status.services) == "array" {
                    for s in svc.status.services {
                        s.instance = #{
                            name: svc.metadata.name,
                            namespace: svc.metadata.namespace,
                        };
                        s["package"] = #{
                            category: svc.spec.category,
                            name: svc.spec["package"],
                        };
                        if "tenant" in s.keys() {
                            if s.tenant == tenant {
                                s.remove("tenant");
                                if "service" in s.keys() {
                                    s += s.service;
                                    s.remove("service");
                                }
                                lst.push(s);
                            }
                        } else {
                            if "service" in s.keys() {
                                s += s.service;
                                s.remove("service");
                            }
                            lst.push(s);
                        }
                    }
                }
            }
        } else {
            for svc in svcs {
                if type_of(svc.status) == "map" && type_of(svc.status.services) == "array" {
                    for s in svc.status.services {
                        s.instance = #{
                            name: svc.metadata.name,
                            namespace: svc.metadata.namespace,
                        };
                        s["package"] = #{
                            category: svc.spec.category,
                            name: svc.spec["package"],
                        };
                        if ! ("tenant" in s.keys()) {
                            if "service" in s.keys() {
                                s += s.service;
                                s.remove("service");
                            }
                            lst.push(s);
                        }
                    }
                }
            }
        }
        cluster_config["services"] = lst;
    } catch (e) {
        switch type_of(e) {
            "string" => host::log_warn(e),
            _ => host::log_warn(core::json_encode(e))
        }
    }

    #{
        cluster: cluster_config,
        controller: controller,
        instance: #{
            name: instance.metadata.name,
            namespace: instance.metadata.namespace,
            "package": #{
                name: instance.spec["package"],
                jukebox: instance.spec.jukebox,
                category: instance.spec.category,
                app_version: pkg.metadata.app_version,
                options: pkg.options,
                "type": pkg.metadata["type"],
                backup_affinity: pkg.metadata.backup_affinity,
            },
            labels: #{
                "app.kubernetes.io/name": instance.spec["package"],
                "app.kubernetes.io/instance": instance.metadata.name,
                "app.kubernetes.io/managed-by": "vynil",
                "vynil.solidite.fr/owner-namespace": instance.metadata.namespace,
                "vynil.solidite.fr/owner-category": instance.spec.category,
                "vynil.solidite.fr/owner-type": pkg.metadata["type"],
            },
            selector: #{
                "app.kubernetes.io/name": instance.spec["package"],
                "app.kubernetes.io/instance": instance.metadata.name,
            },
            images: pkg.images,
            resources: pkg.resources,
            current: current,
            requested: args.tag,
            appslug: appslug(instance.spec["package"], instance.metadata.name)
        },
        values: get_values(instance.spec.options, defaults),
        defaults: defaults,
        package_dir: args.package_dir,
        config_dir: args.config_dir,
    }
}
