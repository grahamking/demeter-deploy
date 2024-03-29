use crate::progress_message::Progress;
use crate::remote::Remote;
use crate::ssh::{LogLevel, MockSSH, SSH};
use crossbeam_channel::{unbounded, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

pub struct SSHManager {
    primary: Box<dyn Remote>,
    upload_sender: Option<Sender<(String, String)>>,
    upload_workers: Vec<thread::JoinHandle<()>>,
}

impl SSHManager {
    pub fn new(
        host: &str,
        username: &str,
        log_level: LogLevel,
        num_workers: usize,
        progress_sender: Sender<Progress>,
    ) -> anyhow::Result<SSHManager> {
        let primary = SSH::new(host, username, log_level, progress_sender.clone())?;
        let (upload_sender, upload_receiver) = unbounded::<(String, String)>();
        let mut upload_workers = Vec::with_capacity(num_workers);

        // one ssh at a time
        let ssh_lock = Arc::new(Mutex::new(()));

        // start upload workers
        let host = host.to_string();
        let username = username.to_string();
        for tid in 1..=num_workers {
            let host = host.clone();
            let username = username.clone();
            let upload_receiver = upload_receiver.clone();
            let progress_sender = progress_sender.clone();
            let ssh_lock = ssh_lock.clone();
            let thread_handle = thread::Builder::new()
                .name(format!("upload_worker_{tid}"))
                .spawn(move || {
                    let guard = ssh_lock.lock();
                    let ssh = SSH::new(&host, &username, log_level, progress_sender).unwrap();
                    drop(guard);
                    for (src, dst) in upload_receiver {
                        ssh.upload(&src, &dst).unwrap();
                    }
                })?;
            upload_workers.push(thread_handle);
        }

        Ok(SSHManager {
            primary: Box::new(primary),
            upload_sender: Some(upload_sender),
            upload_workers,
        })
    }

    // Replaces self with a mocked SSH connection, so we can report what would really happen
    pub fn switch_to_dry_run(self) -> Self {
        self.stop();
        SSHManager {
            primary: Box::new(MockSSH {}),
            upload_sender: None,
            upload_workers: Vec::new(),
        }
    }

    // wait until all upload workers are done
    pub fn stop(mut self) {
        drop(self.upload_sender.take());
        for thread_handle in self.upload_workers {
            thread_handle.join().unwrap();
        }
    }
}

impl Remote for SSHManager {
    fn run_remote_cmd(&self, cmd: &str) -> anyhow::Result<(String, i32)> {
        self.primary.run_remote_cmd(cmd)
    }

    fn mkdir(&self, dir: &str, perms: u32) -> anyhow::Result<()> {
        self.primary.mkdir(dir, perms)
    }

    fn delete(&self, path: &str) -> anyhow::Result<()> {
        self.primary.delete(path)
    }

    // Special function to upload the helper binary.
    // Takes bytes instead of a src filename and
    // uploads using the primary connection, not the thread pool.
    // This makes upload blocking and does not output progress.
    fn upload_bytes(&self, src_bytes: &[u8], dst: &str) -> anyhow::Result<()> {
        self.primary.upload_bytes(src_bytes, dst)
    }

    // upload a file to remote.
    // queues for upload and returns immediately.
    // call stop() once all the uploads are queued to wait for completion.
    fn upload(&self, src: &str, dst: &str) -> anyhow::Result<()> {
        match &self.upload_sender {
            // normal multi-threaded mode
            Some(sender) => {
                sender.send((src.to_string(), dst.to_string()))?;
                Ok(())
            }
            // dry-run mode
            None => self.primary.upload(src, dst),
        }
    }
}
