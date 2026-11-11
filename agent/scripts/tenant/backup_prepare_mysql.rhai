fn run(context) {
    import_run("backup_prepare_mysql_pre", context);
    for mysql in context.mysql_list {
        host::log_info(`Dumping mysql target: ${mysql}`);
        let dbname = host::get_env(`${mysql}_dbname`);
        let rc = if dbname == "*" {
            host::shell_run("mysqldump --skip-ssl --all-databases --host=\"${"+mysql+"_host}\" --user=\"${"+mysql+"_username}\" --password=\"${"+mysql+"_password}\" --no-create-db --add-drop-table > /backup/mysql_"+mysql+".sql")
        } else {
            host::shell_run("mysqldump --skip-ssl --host=\"${"+mysql+"_host}\" --user=\"${"+mysql+"_username}\" --password=\"${"+mysql+"_password}\" --databases ${"+mysql+"_dbname} --no-create-db --add-drop-table > /backup/mysql_"+mysql+".sql")
        };
        if rc != 0 {
            throw `mysqldump failed for ${mysql}`;
        }
    }
    import_run("backup_prepare_mysql_post", context);
}
