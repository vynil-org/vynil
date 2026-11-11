fn template(instance, context) {
    context.namespace.ha = context.cluster.ha;
    context.namespace.use_backup = false;
    context.namespace.name = instance.metadata.namespace;
    context
}
fn run(instance, context) {
    context = template(instance, context);
    let ns_api = k8s_resource("Namespace");
    try {
        let prefix = "vynil.solidite.fr/";
        let ns = ns_api.get(instance.metadata.namespace);
        let annotations = ns.metadata.annotations;
        if type_of(annotations) == "map" {
            for k in annotations.keys().filter(|k| k.starts_with(prefix)) {
                let opt = k;
                opt.remove(prefix);
                try {
                    context.namespace[opt] = core::json_decode(annotations[k]);
                } catch (e) {
                    switch type_of(e) {
                        "string" => host::log_warn(e),
                        _ => host::log_warn(core::json_encode(e))
                    }
                }
            }
        }

    } catch (e) {
        switch type_of(e) {
            "string" => host::log_warn(e),
            _ => host::log_warn(core::json_encode(e))
        }
    }
    context
}