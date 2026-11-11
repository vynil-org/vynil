fn run(context) {
    import_run("backup_prepare_secret_pre", context);
    for secret in context.secret_list {
        host::log_info(`Saving secret: ${secret}`);
        host::create_dir(`/backup/${secret}`);
        for file in host::read_dir(`/secrets/${secret}`) {
            let base = core::basename(file);
            if ! base.starts_with(".") {
                host::file_copy(file, `/backup/${secret}/${base}`);
            }
        }
    }
    import_run("backup_prepare_secret_post", context);
}
