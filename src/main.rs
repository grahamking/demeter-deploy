use core::arch::x86_64::_mm_crc32_u64;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{BufReader, Read};
use std::path;
use std::process;
use std::thread;

use clap::arg;

mod ssh;
use ssh::{MockSSH, Remote, SSH};

const DESC: &str = r#"Example: rcple /home/graham/myfiles graham@myhost.com:/var/www/myfiles
The format is intentionally the same as `scp`."#;

const CRC32: u64 = 0xFFFFFFFF;
const HELPER: &str = "/home/graham/src/rcple/asm/rcple-h";

fn main() -> Result<(), anyhow::Error> {
    let args = clap::Command::new("rcple src_dir user@host:remote_dst_dir")
        .about(DESC)
        .arg(arg!(-v --verbose "Debug level output").required(false))
        .arg(arg!(-d --"dry-run" "Show what we would do without doing it").required(false))
        .arg(arg!([src_dir] "Local directory to copy from").required(true))
        .arg(arg!([remote] "Remote to copy to in format user@host:/dir/").required(true))
        .get_matches();

    let verbose = args.is_present("verbose");
    let is_dry_run = args.is_present("dry-run");

    // clap makes sure these two are populated, we don't need to check
    let mut src_dir = args.value_of("src_dir").unwrap().to_string();
    let remote_str = args.value_of("remote").unwrap().to_string();
    if !src_dir.ends_with('/') {
        src_dir.push('/');
    }

    // split remote string into username, hostname and path

    let mut from_env = "".to_string();
    let (username, remote_str) = remote_str.split_once('@').unwrap_or_else(|| {
        from_env = env::var("USERNAME").expect("missing @ in remote part and $USERNAME not set");
        (&from_env, &remote_str)
    });
    let (hostname, dst_dir) = remote_str
        .split_once(':')
        .expect("missing hostname (':' separator) in remote part");
    let mut dst_dir = dst_dir.to_string();
    if !dst_dir.ends_with('/') {
        dst_dir.push('/');
    }

    // check we have the helper binary
    if !path::Path::new(HELPER).exists() {
        eprintln!("Helper binary '{}' not found.", HELPER);
        return Ok(());
    };

    // start local check in the background

    let src_dir_for_local = src_dir.clone();
    let local_thread = thread::Builder::new()
        .name("local checksum".to_string())
        .spawn(move || checksum_dir(src_dir_for_local.into()))?;

    // remote

    if verbose {
        println!("Using libssh {}", SSH::version());
    }
    let real_ssh = SSH::new(hostname, username, ssh::LogLevel::WARNING)?;
    let mut ssh: Box<dyn Remote> = Box::new(real_ssh);

    ssh.upload(HELPER, "/tmp/rcple-h")?;

    let output = ssh.run_remote_cmd(&format!("/tmp/rcple-h {}", dst_dir))?;

    let remote: HashMap<&str, u32> = output
        .lines()
        .map(|l| {
            l.split_once(':')
                .map_or(("", 0), |(k, v)| (k, v.parse().unwrap()))
        })
        .collect();

    // join local check

    let local = match local_thread.join() {
        Ok(checksum_dir_ret) => match checksum_dir_ret {
            Ok(c) => c,
            Err(err) => {
                eprintln!("Error on local dir {}: {}", src_dir, err);
                process::exit(1);
            }
        },
        Err(err) => {
            // thread panic
            panic!("{:?}", err);
        }
    };

    // compare

    let mut upload = Vec::new();
    for (filename, l_crc32) in local.iter() {
        match remote.get(filename.as_str()) {
            None => {
                upload.push(filename);
            }
            Some(r_crc32) if r_crc32 != l_crc32 => {
                upload.push(filename);
            }
            _ => {} // they are the same
        }
    }

    let mut delete = Vec::new();
    let mut remote_dirs = HashSet::with_capacity(64);
    for (filename, _) in remote {
        let p = path::PathBuf::from(filename);
        if let Some(dir) = p.parent() {
            remote_dirs.insert(dir.to_path_buf());
        }

        if !local.contains_key(filename) {
            delete.push(filename);
        }
    }

    if verbose {
        println!("Upload: {:?}", upload);
        println!("Delete: {:?}", delete);
    }

    if is_dry_run {
        ssh = Box::new(MockSSH {});
    }

    // action

    for filename in upload {
        // Do we need to make the parent dir(s)?
        let p = path::PathBuf::from(filename);
        if let Some(dir) = p.parent() {
            if !dir.as_os_str().is_empty() && !remote_dirs.contains(dir) {
                if verbose {
                    println!("mkdir remote: {}", dir.display());
                }
                for component in dir
                    .ancestors()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .skip(1)
                {
                    ssh.mkdir(&format!("{dst_dir}{}", component.display()), 0o755)?;
                    remote_dirs.insert(component.to_path_buf());
                }
            }
        }
        ssh.upload(
            &format!("{src_dir}{filename}"),
            &format!("{dst_dir}{filename}"),
        )?;
    }

    for filename in delete {
        ssh.delete(&format!("{dst_dir}{}", filename))?;
    }

    Ok(())
}

fn checksum_dir(path: path::PathBuf) -> Result<HashMap<String, u32>, anyhow::Error> {
    let path_len = path.to_string_lossy().len();
    let mut out = HashMap::with_capacity(64);
    let mut dirs = vec![path];

    while !dirs.is_empty() {
        let next_dir = dirs.pop().unwrap();
        for entry in fs::read_dir(next_dir)? {
            let file = entry?;
            let filename = file.file_name().to_string_lossy().into_owned();
            if filename.starts_with('.') {
                continue;
            }
            let file_type = file.file_type()?;
            if file_type.is_dir() {
                dirs.push(file.path());
            } else {
                let f = fs::File::open(file.path())?;
                let mut f = BufReader::new(f);
                let mut ubuf = [0; 8];

                let mut checksum = CRC32;
                loop {
                    ubuf.fill(0);
                    match f.read(&mut ubuf) {
                        Ok(0) => {
                            break;
                        }
                        Ok(_) => {
                            checksum = unsafe { _mm_crc32_u64(checksum, u64::from_le_bytes(ubuf)) };
                        }
                        Err(err) => {
                            return Err(err.into());
                        }
                    }
                }
                out.insert(
                    file.path()
                        .to_string_lossy()
                        .get(path_len..)
                        .unwrap()
                        .to_string(),
                    (checksum & CRC32) as u32,
                );
            }
        }
    }
    Ok(out)
}

/* TODO
 - upload multiple files at once
    try it
 - For all the SSHResult returns, if ERROR call ssh_get_error like on ssh_connect
    For sftp maybe call sftp_get_error
 - flag to skip dot (hidden) files (or to include them)
 - nice output showing
   - how many files done / remain
   - files uploading chunk by chunk
 - remote in a thread?
 - rcpl-h (asm) if file > size crc in a thread (with max thread as num CPUs)
 - rcpl-h (asm) make it as small as possible
*/
