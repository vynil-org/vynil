fn run(context) {
    import_run("backup_prepare_postgresql_pre", context);
    for pg in context.pg_list {
        host::log_info(`Dumping postgresql target: ${pg}`);
        let dbname = host::get_env(`${pg}_dbname`);
        let rc = if dbname == "*" {
            host::shell_run("export PGPASSWORD=\"${"+pg+"_password}\";pg_dumpall -h \"${"+pg+"_host}\" -U \"${"+pg+"_username}\" --exclude-database=postgres \"--exclude-database=template*\" --exclude-database=streaming_replica --clean -f /backup/postgresql_"+pg+".sql")
        } else {
            host::shell_run("export PGPASSWORD=\"${"+pg+"_password}\";pg_dump -h \"${"+pg+"_host}\" -d \"${"+pg+"_dbname}\" -U \"${"+pg+"_username}\" --clean -f /backup/postgresql_"+pg+".sql")
        };
        if rc != 0 {
            throw `pg_dump failed for ${pg}`;
        }
        if dbname == "*" {
            host::shell_run("sed -i -E '/^(CREATE|ALTER|DROP) ROLE (streaming_replica|postgres)/d' /backup/postgresql_"+pg+".sql")
        }
    }
    import_run("backup_prepare_postgresql_post", context);
}
