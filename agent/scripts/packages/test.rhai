import "build" as build;
import "context" as ctx;
import "install" as install;

fn run(instance, args) {
    host::log_info(`Building to ${args.temp}`);
    build::build(args);
    args.package_dir = args.temp;
    args.tag = if instance.status != () && instance.status.tag != () {instance.status.tag} else {"0.0.1"};
    host::log_info(`Generating context for ${args.namespace} ${args.instance}`);
    let context = ctx::run(instance, args);
    host::log_info(`Installing ${args.namespace} ${args.instance}`);
    install::run(instance, context);
}