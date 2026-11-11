fn run(context) {
    import_run("backup_prune_pre", context);
    host::log_info("Pruning restic backup");
    let rc = host::shell_run(`export RESTIC_REPOSITORY="${context.s3_url}";restic forget ${context.restic_args} --keep-daily "${context.max_dayly}" --keep-weekly "${context.max_weekly}" --keep-monthly "${context.max_monthly}" --keep-yearly "${context.max_yearly}" --prune`);
    if rc != 0 {
        throw `restic prune FAILED returning ${rc}`;
    }
    import_run("backup_prune_post", context);
}
