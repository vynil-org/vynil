import "backup_context" as bctx;
import "install_from_dir" as dir;
import "wait" as wait;
import "yaml_docs" as yd;

fn run(instance, context) {
    let ctx = import_run("init_from_pre", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    try {
        let ctx = import_run("init_from_prepare", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
        let hbs = new_hbs();
        hbs.register_partial_dir(context.template_dir);
        if is_dir(`${context.package_dir}/pods`) {
            hbs.register_partial_dir(`${context.package_dir}/pods`);
        }
        let all = yd::decode(hbs.render_from("{{> init_job.yaml }}", bctx::run(instance, context, true)));
        let applied_objs = [];
        for obj in all {
            let to_apply = true;
            if obj.kind == "Job" {
                try {
                    let api = k8s_resource("Job", instance.metadata.namespace);
                    if api.exist() {
                        let lst = api.list().items.filter(|j| j.metadata.name == obj.metadata.name);
                        to_apply = lst.len() < 1;
                    }
                } catch (e) {
                    switch type_of(e) {
                        "string" => log_warn(e),
                        _ => log_warn(json_encode(e))
                    }
                }
            }
            if to_apply {
                applied_objs += dir::install_obj(obj, true, instance.metadata.namespace, false);
            }
        }
        wait::all(applied_objs, 20*60);
    } catch (e) {
        switch type_of(e) {
            "string" => instance.set_status_init_failed(e),
            _ => instance.set_status_init_failed(json_encode(e))
        }
        throw;
    }
    ctx = import_run("init_from_post", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    context
}
