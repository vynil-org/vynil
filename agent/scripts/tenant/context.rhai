import "build_context" as build;
fn template(instance, args) {
    let context = build::template(instance, args);
    context["template_dir"] = args.template_dir;
    context["agent_image"] = args.agent_image;
    let ctx = import_template("context_pre", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    context["tenant"] = #{
        name: "client",
        namespaces: ["client-system", instance.metadata.namespace],
        maintenance_start_hour: "1",
        maintenance_start_minut: "0",
    };
    ctx = import_template("context_tenant", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    context["extra"] = import_template("context_extra", instance, context);
    ctx = import_template("context_post", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    context
}
fn run(instance, args) {
    let context = build::run(instance, args);
    context["template_dir"] = args.template_dir;
    context["agent_image"] = args.agent_image;
    let ctx = import_run("context_pre", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    let name = instance.get_tenant_name();
    let nss = instance.get_tenant_namespaces();
    context["tenant"] = #{
        name: name,
        namespaces: nss,
        services: [],
        maintenance_start_hour: "1",
        maintenance_start_minut: "0",
    };
    for ns in nss {
        try {
            let svcs = k8s_resource("TenantInstance", ns).list().items;
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
                        if "service" in s {
                            s += s.service;
                            s.remove("service");
                        }
                        context["tenant"]["services"].push(s);
                    }
                }
            }
        } catch (e) {
            switch type_of(e) {
                "string" => host::log_warn(e),
                _ => host::log_warn(core::json_encode(e))
            }
        }
    }
    ctx = import_run("context_tenant", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    context["extra"] = import_run("context_extra", instance, context);
    ctx = import_run("context_post", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    ctx = import_run("context_after", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    context
}
