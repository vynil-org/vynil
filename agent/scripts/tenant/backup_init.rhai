fn run(context) {
    import_run("backup_init_pre", context);
    host::log_info("Checking for restic repository existance");
    let rc = host::shell_run(`export RESTIC_REPOSITORY="${context.s3_url}";restic ${context.restic_args} cat config`);
    if rc == 10 {
        host::log_info("Create restic repository");
        host::shell_run(`export RESTIC_REPOSITORY="${context.s3_url}";restic init ${context.restic_args}`);
    }
    import_run("backup_init_post", context);
}
