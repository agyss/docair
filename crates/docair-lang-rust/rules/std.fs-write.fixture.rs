// Fixture for std.fs-write.yaml. Every `//>` line is a fact the rule must
// find, and it must find no others.

pub fn save(path: &Path, json: &str) {
    std::fs::write(path, json).unwrap();
}

pub fn out_dir(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
}

pub fn patch() {
    fs::write("docair.patch", "").unwrap();
}

//> writes file: from fn save
//> writes file: from fn out_dir
//> writes file:docair.patch from fn patch
