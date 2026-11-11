fn dir_exts(src, dst, exts, recursiv) {
    host::create_dir(dst);
    for item in host::read_dir(src) {
        let base = core::basename(item);
        if host::is_file(item) {
            if exts.some(|ext| item.ends_with(ext)) {
                host::file_copy(item, dst+"/"+base);
            }
        } else if recursiv && host::is_dir(item) {
            dir_exts(src+"/"+base, dst+"/"+base, exts, recursiv);
        }
    }
}
fn dir_exts(src, dst, exts) { dir_exts(src, dst, exts, false); }
fn dir_all(src, dst, recursiv) {
    host::create_dir(dst);
    for item in host::read_dir(src) {
        let base = core::basename(item);
        if host::is_file(item) {
            host::file_copy(item, dst+"/"+base);
        } else if recursiv && host::is_dir(item) {
            dir_all(src+"/"+base, dst+"/"+base, recursiv);
        }
    }
}
fn dir_all(src, dst) { dir_all(src, dst, false); }
