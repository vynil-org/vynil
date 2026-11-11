import "install_from_dir" as dir;
import "wait" as wait;
fn run(instance, context) {
    let ctx = import_run("install_scalables_pre", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    try {
        let applied_objs = dir::install(instance, context, `${context.package_dir}/scalables`, true, false);
        // TODO: scale a target
        let added = import_run("install_scalables_add", instance, context);
        switch type_of(added) {
            "map" => {
                if dir::validate_add_item(added) {
                    applied_objs.push(added);
                }
            },
            "array" => {
                applied_objs += added.filter(|i|dir::validate_add_item(i));
            }
        }
        if instance.status != () && instance.status.scalables != () {
            dir::remove(applied_objs, instance.status.scalables, instance.metadata.namespace);
        }
        let tmp = instance.set_status_scalables(applied_objs);
        wait::all(applied_objs);
    } catch (e) {
        switch type_of(e) {
            "string" => instance.set_status_scalable_failed(e),
            _ => instance.set_status_scalable_failed(core::json_encode(e))
        }
        throw;
    }
    ctx = import_run("install_scalables_post", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    context
}
