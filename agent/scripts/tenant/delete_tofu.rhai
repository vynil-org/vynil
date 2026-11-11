import "tofu" as tf;
import "tofu_gen" as tfg;

fn run(instance, context) {
    let ctx = import_run("delete_tofu_pre", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    let orig = instance.get_tfstate();
    if orig != "" {
        host::file_write(`${context.package_dir}/tofu/terraform.tfstate`, orig);
    }
    tfg::gen_files(context);
    tf::run_init(`${context.package_dir}/tofu`);
    tf::run_destroy(`${context.package_dir}/tofu`);
    ctx = import_run("delete_tofu_post", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    context
}
