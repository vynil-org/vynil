import "backup_context" as ctx;
fn run(args) {
    let context = ctx::from_args(args);
    host::log_info(`Starting backup using target: ${context.s3_url}`);

    import_run("backup_pre", context);
    if host::is_file(`${args.package_dir}/scripts/backup.sh`) {
        let rc = host::shell_run(`export RESTIC_REPOSITORY="${context.s3_url}";${context.package_dir}/scripts/backup.sh`);
        if rc != 0 {
            throw `${context.package_dir}/scripts/backup.sh FAILED returning ${rc}`;
        } else {
            import_run("backup_post", context);
            return rc;
        }
    }
    import_run("backup_init", context);
    import_run("maintenance_start", context);
    if host::is_file(`${args.package_dir}/scripts/backup_prepare.sh`) {
        let rc = host::shell_run(`export RESTIC_REPOSITORY="${context.s3_url}";${context.package_dir}/scripts/backup_prepare.sh`);
        if rc != 0 {
            throw `${context.package_dir}/scripts/backup_prepare.sh FAILED returning ${rc}`;
        }
    } else {
        if host::is_file(`${args.package_dir}/scripts/backup_prepare_secret.sh`) {
            let rc = host::shell_run(`export RESTIC_REPOSITORY="${context.s3_url}";${context.package_dir}/scripts/backup_prepare_secret.sh`);
            if rc != 0 {
                throw `${context.package_dir}/scripts/backup_prepare_secret.sh FAILED returning ${rc}`;
            }
        } else {
            import_run("backup_prepare_secret", context);
        }
        if host::is_file(`${args.package_dir}/scripts/backup_prepare_postgresql.sh`) {
            let rc = host::shell_run(`export RESTIC_REPOSITORY="${context.s3_url}";${context.package_dir}/scripts/backup_prepare_postgresql.sh`);
            if rc != 0 {
                throw `${context.package_dir}/scripts/backup_prepare_postgresql.sh FAILED returning ${rc}`;
            }
        } else {
            import_run("backup_prepare_postgresql", context);
        }
        if host::is_file(`${args.package_dir}/scripts/backup_prepare_mysql.sh`) {
            let rc = host::shell_run(`export RESTIC_REPOSITORY="${context.s3_url}";${context.package_dir}/scripts/backup_prepare_mysql.sh`);
            if rc != 0 {
                throw `${context.package_dir}/scripts/backup_prepare_mysql.sh FAILED returning ${rc}`;
            }
        } else {
            import_run("backup_prepare_mysql", context);
        }
    }
    if host::is_file(`${args.package_dir}/scripts/backup_before.sh`) {
        let rc = host::shell_run(`export RESTIC_REPOSITORY="${context.s3_url}";${context.package_dir}/scripts/backup_before.sh`);
        if rc != 0 {
            throw `${context.package_dir}/scripts/backup_before.sh FAILED returning ${rc}`;
        }
    }
    import_run("backup_run", context);
    import_run("maintenance_stop", context);
    import_run("backup_check", context);
    //import_run("backup_prune", context);
    import_run("backup_post", context);
}
