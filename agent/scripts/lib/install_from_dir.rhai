import "yaml_docs" as yd;
fn get_first() { [
    "Tenant",
    "Namespace",
    "ServiceAccount",
    "ClusterRole",
    "ClusterRoleBinding",
    "Role",
    "RoleBinding",
    "Secret",
    "ConfigMap",
    "PersistentVolume",
    "PersistentVolumeClaim",
    "NetworkPolicy"
] }
fn get_last() { [
    "Pod",
    "Job",
    "CronJob",
    "DaemonSet",
    "Deployment",
    "ReplicaSet",
    "StatefulSet",
    "Service",
    "Ingress",
    "APIService",
    "MutatingWebhookConfiguration",
    "ValidatingAdmissionPolicy",
    "ValidatingAdmissionPolicyBinding",
    "ValidatingWebhookConfiguration",
] }

fn get_objects(context, dir) {
    let hbs = new_hbs();
    hbs.register_partial_dir(context.config_dir);
    hbs.register_partial_dir(`${context.package_dir}/handlebars/partials`);
    hbs.register_helper_dir(`${context.package_dir}/handlebars/helpers`);
    let ret = [];
    for file in read_dir(dir) {
        let objects = [];
        try {
            objects = if file.ends_with(".yaml") || file.ends_with(".yml") {
                yd::decode(file_read(file))
            } else if file.ends_with(".yaml.hbs") || file.ends_with(".yml.hbs") {
                yd::decode(hbs.render_named(file, file_read(file), context))
            } else {[]};
        } catch(e) {
            log_error(`While parsing ${file} :`);
            throw;
        }
        for obj in objects.filter(|obj| type_of(obj) != "map" || ! obj.keys().contains("kind") || ! obj.keys().contains("metadata") || type_of(obj.metadata) != "map") {
            log_warn(`No kind for an object in file ${file}`);
            log_debug(yaml_encode(obj));
        }
        ret += objects.filter(|obj| type_of(obj) == "map" && obj.keys().contains("kind") && obj.keys().contains("metadata") && type_of(obj.metadata) == "map");
    }
    ret
}

fn install_obj(obj, allow_cluster, namespace, force_ns) {
    let applied = [];
    log_debug(`Applying:`);
    log_debug(yaml_encode(obj));
    let ns = if (force_ns) {
        namespace
    } else {
        if obj.metadata.namespace != () {obj.metadata.namespace} else {namespace}
    };
    let api = k8s_resource(obj.apiVersion, obj.kind, ns);
    if api.exist() {
        if force_ns && api.scope != "cluster" {
            obj.metadata.namespace = ns;
        }
        if api.scope == "cluster" {
            if allow_cluster {
                log_info(`Applying ${obj.kind} ${obj.metadata.name}`);
                api.apply(obj.metadata.name, obj);
                applied.push(#{
                    api_version: obj.apiVersion,
                    apiVersion: obj.apiVersion,
                    kind: obj.kind,
                    name: obj.metadata.name
                });
            } else {
                log_warn(`Ignoring ${obj.kind} ${obj.metadata.name} as it is a clusterwide resource type.`);
            }
        } else {
            log_info(`Applying ${obj.kind} ${ns}/${obj.metadata.name}`);
            api.apply(obj.metadata.name, obj);
            applied.push(#{
                api_version: obj.apiVersion,
                apiVersion: obj.apiVersion,
                kind: obj.kind,
                name: obj.metadata.name,
                namespace: ns
            });
        }
    } else {
        log_debug(yaml_encode(obj));
        throw `kind: ${obj.kind} is unknown from the apiServer.`;
    }
    applied
}
fn install(instance, context, dir, allow_cluster, force_ns) {
    let all = get_objects(context, dir);
    let applied_objs = [];
    for k in get_first() {
        for obj in all.filter(|o| o.kind==k) {
            applied_objs += install_obj(obj, allow_cluster, instance.metadata.namespace, force_ns);
        }
    }
    for obj in all.filter(|o| !get_first().contains(o.kind) && !get_last().contains(o.kind)) {
        applied_objs += install_obj(obj, allow_cluster, instance.metadata.namespace, force_ns);
    }
    for k in get_last() {
        for obj in all.filter(|o| o.kind==k) {
            applied_objs += install_obj(obj, allow_cluster, instance.metadata.namespace, force_ns);
        }
    }
    applied_objs
}

fn install_allowed_obj(obj, allowed, namespace) {
    let applied = [];
    log_debug(`Applying:`);
    log_debug(yaml_encode(obj));
    let ns = if (obj.metadata.namespace != () && obj.metadata.namespace in allowed) {
        obj.metadata.namespace
    } else {
        namespace
    };
    let api = k8s_resource(obj.apiVersion, obj.kind, ns);
    if api.exist() {
        if api.scope != "cluster" {
            obj.metadata.namespace = ns;
            log_info(`Applying ${obj.kind} ${ns}/${obj.metadata.name}`);
            api.apply(obj.metadata.name, obj);
            applied.push(#{
                api_version: obj.apiVersion,
                apiVersion: obj.apiVersion,
                kind: obj.kind,
                name: obj.metadata.name,
                namespace: ns
            });
        } else {
            log_warn(`Ignoring ${obj.kind} ${obj.metadata.name} as it is a clusterwide resource type.`);
        }
    } else {
        log_debug(yaml_encode(obj));
        throw `kind: ${obj.kind} is unknown from the apiServer.`;
    }
    applied
}

fn install_allowed(instance, context, dir, allowed) {
    // Just like template but output apply the objects
    let all = get_objects(context, dir);
    let applied_objs = [];
    for k in get_first() {
        for obj in all.filter(|o| o.kind==k) {
            applied_objs += install_allowed_obj(obj, allowed, instance.metadata.namespace);
        }
    }
    for obj in all.filter(|o| !get_first().contains(o.kind) && !get_last().contains(o.kind)) {
        applied_objs += install_allowed_obj(obj, allowed, instance.metadata.namespace);
    }
    for k in get_last() {
        for obj in all.filter(|o| o.kind==k) {
            applied_objs += install_allowed_obj(obj, allowed, instance.metadata.namespace);
        }
    }
    applied_objs
}

fn remove(applied, previous, default_ns) {
    let deleted = [];
    for old in previous {
        if ! applied.some(|n| n.kind == old.kind && n.namespace == old.namespace && n.name == old.name) {
            let ns = if old.namespace != () {old.namespace} else {default_ns};
            let api = if old.api_version != () {
                k8s_resource(old.api_version, old.kind, ns)
            } else if old.apiVersion != () {
                k8s_resource(old.apiVersion, old.kind, ns)
            } else {
                k8s_resource(old.kind, ns)
            };
            if api.exist() {
                try {
                    let o = api.get_obj(old.name);
                    o.delete();
                    deleted.push(o);
                } catch (e) {
                    switch type_of(e) {
                        "string" => if ! e.contains("NotFound") { log_warn(e) },
                        _ => log_warn(json_encode(e))
                    }
                }
            }
        }
    }
    for d in deleted {
        log_info(`Waiting for ${d.original_kind} ${d.metadata.name} to be deleted`);
        d.wait_deleted(60*5);
    }
}

fn validate_add_item(item) {
    switch type_of(item) {
        "map" => {
            let cp = item;
            [["kind", "name", "namespace"],["kind", "name"]].contains(item.keys()) &&
            item.keys().reduce(|acc, k| (type_of(cp[k])=="string") && acc, true)
        },
        _ => false
    }
}

fn template(instance, context, dir) {
    // Just like install_allowed but output the objects
    let all = get_objects(context, dir);
    for k in get_first() {
        for obj in all.filter(|o| o.kind==k) {
            print("---");
            print(yaml_encode(obj));
        }
    }
    for obj in all.filter(|o| !get_first().contains(o.kind) && !get_last().contains(o.kind)) {
        print("---");
        print(yaml_encode(obj));
    }
    for k in get_last() {
        for obj in all.filter(|o| o.kind==k) {
            print("---");
            print(yaml_encode(obj));
        }
    }
}
