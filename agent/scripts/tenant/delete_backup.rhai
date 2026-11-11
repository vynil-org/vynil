import "install_from_dir" as dir;

fn run(instance, context) {
    let ctx = import_run("delete_backup_pre", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    let deleted = [];
    try {
        let ctx = import_run("delete_backup_prepare", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
    } catch (e) {
        switch type_of(e) {
            "string" => host::log_warn(e),
            _ => host::log_warn(core::json_encode(e))
        }
    }
    try {
        let api = k8s_resource("CronJob", instance.metadata.namespace);
        if api.exist() {
            try {
                let o = api.get_obj(`${context.instance.appslug}-backups`);
                o.delete();
                deleted.push(o);
            } catch (e) {
                switch type_of(e) {
                    "string" => if ! e.contains("NotFound") { host::log_warn(e) },
                    _ => host::log_warn(core::json_encode(e))
                }
            }
        }
    } catch (e) {
        switch type_of(e) {
            "string" => host::log_warn(e),
            _ => host::log_warn(core::json_encode(e))
        }
    }
    try {
        let api = k8s_resource("Job", instance.metadata.namespace);
        if api.exist() {
            for job in api.list(`app.kubernetes.io/component=backup,app.kubernetes.io/instance=${instance.metadata.name},app.kubernetes.io/name=${context.instance["package"].name}`).items {
                try {
                    let o = api.get_obj(job.metadata.name);
                    o.delete();
                    deleted.push(o);
                } catch (e) {
                    switch type_of(e) {
                        "string" => host::log_warn(e),
                        _ => host::log_warn(core::json_encode(e))
                    }
                }
            }
            for job in api.list(`app.kubernetes.io/component=restore,app.kubernetes.io/instance=${instance.metadata.name},app.kubernetes.io/name=${context.instance["package"].name}`).items {
                try {
                    let o = api.get_obj(job.metadata.name);
                    o.delete();
                    deleted.push(o);
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
    try {
        let api = k8s_resource("RoleBinding", instance.metadata.namespace);
        if api.exist() {
            try {
                let o = api.get_obj(`${context.instance.appslug}-backups`);
                o.delete();
                deleted.push(o);
            } catch (e) {
                switch type_of(e) {
                    "string" => if ! e.contains("NotFound") { host::log_warn(e) },
                    _ => host::log_warn(core::json_encode(e))
                }
            }
        }
    } catch (e) {
        switch type_of(e) {
            "string" => host::log_warn(e),
            _ => host::log_warn(core::json_encode(e))
        }
    }
    try {
        let api = k8s_resource("ConfigMap", instance.metadata.namespace);
        if api.exist() {
            try {
                let o = api.get_obj(`${context.instance.appslug}-backups-scripts`);
                o.delete();
                deleted.push(o);
            } catch (e) {
                switch type_of(e) {
                    "string" => if ! e.contains("NotFound") { host::log_warn(e) },
                    _ => host::log_warn(core::json_encode(e))
                }
            }
        }
    } catch (e) {
        switch type_of(e) {
            "string" => host::log_warn(e),
            _ => host::log_warn(core::json_encode(e))
        }
    }
    for d in deleted {
        host::log_info(`Waiting for ${d.original_kind} ${d.metadata.name} to be deleted`);
        d.wait_deleted(60*5);
    }
    ctx = import_run("delete_backup_post", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    context
}
