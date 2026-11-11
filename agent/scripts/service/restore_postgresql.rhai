fn run(context) {
    import_run("restore_postgresql_pre", context);
    for pg in context.pg_list {
        host::log_info(`Starting postgreSQL restore for ${pg}`);
        let dbname = host::get_env(`${pg}_dbname`);
        let args = "-h \"${"+pg+"_host}\" -U \"${"+pg+"_username}\"";
        let file = `/backup/postgresql_${pg}.sql`;
        if dbname != "*" {
            args += " -d \"${"+pg+"_dbname}\"";
        }
        if host::is_file(`/backup/postgresql_${pg}_cleaned.sql`) {
            file = `/backup/postgresql_${pg}_cleaned.sql`;
        }
        let rc = host::shell_run("export PGPASSWORD=\"${"+pg+"_password}\";psql "+args+" < "+file);
        if rc != 0 {
            throw `psql failed for ${pg}`;
        }
    }
    import_run("restore_postgresql_post", context);
}
