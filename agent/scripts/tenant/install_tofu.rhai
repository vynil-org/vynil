import "tofu" as tf;
import "tofu_gen" as tfg;
fn run(instance, context) {
    let ctx = import_run("install_tofu_pre", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    let orig = instance.get_tfstate();
    if orig != "" {
        host::file_write(`${context.package_dir}/tofu/terraform.tfstate`, orig);
    }
    try {
        tfg::gen_files(context);
        tf::run_init(`${context.package_dir}/tofu`);
        tf::run_plan(`${context.package_dir}/tofu`);
        tf::run_apply(`${context.package_dir}/tofu`);
        context.tofu = core::json_decode(tf::get_output(`${context.package_dir}/tofu`));
        let state = host::file_read(`${context.package_dir}/tofu/terraform.tfstate`);
        instance.set_tfstate(state);
    } catch (e) {
        let state = if (host::is_file(`${context.package_dir}/tofu/terraform.tfstate`)) {
            host::file_read(`${context.package_dir}/tofu/terraform.tfstate`)
        } else {""};
        switch type_of(e) {
            "string" => instance.set_status_tofu_failed(state, e),
            _ => instance.set_status_tofu_failed(state, core::json_encode(e))
        }
        throw;
    }
    ctx = import_run("install_tofu_post", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    context
}
