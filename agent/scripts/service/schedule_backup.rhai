import "backup_context" as bctx;
import "install_from_dir" as dir;
import "yaml_docs" as yd;

fn run(instance, context) {
    let ctx = import_run("schedule_backup_pre", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    try {
        context = bctx::run(instance, context, false);
        let ctx = import_run("schedule_backup_prepare", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
        let hbs = new_hbs();
        hbs.register_partial_dir(context.template_dir);
        if is_dir(`${context.package_dir}/pods`) {
            hbs.register_partial_dir(`${context.package_dir}/pods`);
        }
        let all = yd::decode(hbs.render_from("{{> cronbackup.yaml }}", context));
        for obj in all {
            dir::install_obj(obj, true, instance.metadata.namespace, false);
        }
    } catch (e) {
        switch type_of(e) {
            "string" => instance.set_status_schedule_backup_failed(e),
            _ => instance.set_status_schedule_backup_failed(json_encode(e))
        }
        throw;
    }
    ctx = import_run("schedule_backup_post", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    context
}
