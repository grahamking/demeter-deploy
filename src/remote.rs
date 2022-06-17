pub trait Remote {
    fn run_remote_cmd(&self, cmd: &str) -> anyhow::Result<(String, i32)>;
    fn mkdir(&self, dir: &str, perms: u32) -> anyhow::Result<()>;
    fn upload(&self, src: &str, dst: &str) -> anyhow::Result<()>;
    fn upload_bytes(&self, src_bytes: &[u8], dst: &str) -> anyhow::Result<()>;
    fn delete(&self, path: &str) -> anyhow::Result<()>;
}
