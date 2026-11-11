fn run(context) {
    import_run("restore_mongodb_pre", context);
    for mongo in context.mongo_list {
        host::log_info(`Starting MongoDB restore for ${mongo}`);
        let dbname = host::get_env(`${mongo}_dbname`);
        let args = "--host=\"${"+mongo+"_host}\" --username=\"${"+mongo+"_username}\" --password=\"${"+mongo+"_password}\" --authenticationDatabase=admin --drop";
        if dbname != "*" {
            args += " --nsInclude=\"${"+mongo+"_dbname}.*\"";
        }
        let rc = host::shell_run("mongorestore "+args+" --archive=/backup/mongodb_"+mongo+".archive");
        if rc != 0 {
            throw `mongorestore failed for ${mongo}`;
        }
    }
    import_run("restore_mongodb_post", context);
}
