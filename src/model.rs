#[derive(Debug)]
pub struct PackageUpdate {
    pub name: String,
    pub arch: String,
    pub old_version: String,
    pub new_version: String,
    pub old_repo: String,
    pub new_repo: String,
    pub download_size: u64,
}

#[derive(Default)]
pub struct SizeInfo {
    pub download: Option<u64>,
    pub net_disk: Option<i64>,
}
