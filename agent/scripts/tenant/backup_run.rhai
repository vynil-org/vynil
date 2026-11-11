fn run(context) {
    import_run("backup_run_pre", context);
    host::log_info("Starting restic backup");
    let rc = host::shell_run(`export RESTIC_REPOSITORY="${context.s3_url}";restic backup /backup ${context.restic_args} --tag "${context.restic_tags}"`);
    if rc != 0 {
        throw `restic backup FAILED returning ${rc}`;
    }
    import_run("backup_run_post", context);
}
