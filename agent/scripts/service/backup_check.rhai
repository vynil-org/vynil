fn run(context) {
    import_run("backup_check_pre", context);
    host::log_info("Checking restic repository");
    let rc = host::shell_run(`export RESTIC_REPOSITORY="${context.s3_url}";restic check ${context.restic_args}`);
    if rc != 0 {
        throw `restic check FAILED returning ${rc}`;
    }
    import_run("backup_check_post", context);
}
