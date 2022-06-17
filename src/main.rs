use core::arch::x86_64::_mm_crc32_u64;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{BufReader, Read};
use std::path;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use clap::arg;
use crossbeam_channel::unbounded;

mod ssh_manager;
use ssh_manager::SSHManager;

mod ssh;
use ssh::SSH;

mod remote;
use remote::Remote;

mod progress_message;
use progress_message::Progress;

mod output;
use output::run_output;

const DESC: &str = r#"Example: rcple /home/graham/myfiles graham@myhost.com:/var/www/myfiles
The format is intentionally the same as `scp`."#;

const CRC32: u64 = 0xFFFFFFFF;
const HELPER_SRC: &str = "/home/graham/src/rcple/asm/rcple-h";
const HELPER_SEP: char = ':';
const DEFAULT_HELPER_DST: &str = "/tmp/rcple-h";

fn main() -> Result<(), anyhow::Error> {
    let args = clap::Command::new("rcple src_dir user@host:remote_dst_dir")
        .about(DESC)
        .arg(arg!(--"dry-run" "Show what we would do without doing it").required(false))
        .arg(arg!(-v --verbose "Debug level output").required(false))
        .arg(arg!(-H --hidden "Include hidden (dot) files").required(false))
        .arg(
            arg!(-w --workers "Number of concurrent SSH connections")
                .required(false)
                .default_value("4"),
        )
        .arg(
            arg!(--"helper-dst" "Full path to upload remote helper binary to")
                .required(false)
                .default_value(DEFAULT_HELPER_DST),
        )
        .arg(arg!([src_dir] "Local directory to copy from").required(true))
        .arg(arg!([remote] "Remote to copy to in format user@host:/dir/").required(true))
        .get_matches();

    let num_workers: usize = args.value_of("workers").unwrap().parse()?;
    let verbose = args.is_present("verbose");
    let is_dry_run = args.is_present("dry-run");
    let is_include_hidden = args.is_present("hidden");
    let helper_dst = args.value_of("helper-dst").unwrap();

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
    if !path::Path::new(HELPER_SRC).exists() {
        eprintln!("Helper binary '{}' not found.", HELPER_SRC);
        process::exit(1);
    };

    let (progress_sender, progress_receiver) = unbounded::<Progress>();

    // start local check in the background

    let src_dir_for_local = src_dir.clone();
    let local_thread = thread::Builder::new()
        .name("local checksum".to_string())
        .spawn(move || checksum_dir(src_dir_for_local.into(), is_include_hidden))?;

    // remote

    if verbose {
        println!("Using libssh {}", SSH::version());
    }
    let mut ssh = match SSHManager::new(
        hostname,
        username,
        ssh::LogLevel::NOLOG,
        num_workers,
        progress_sender.clone(),
    ) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("Could not ssh to '{username}@{hostname}': {err}");
            process::exit(1);
        }
    };

    println!("Gathering information from {hostname}..");
    ssh.upload_primary(HELPER_SRC, helper_dst)?;

    let remote_cmd = &format!("{helper_dst} {dst_dir}");
    let (output, exit_status) = ssh.run_remote_cmd(remote_cmd)?;
    match exit_status {
        0 => {} // success
        x if x < 0 => {
            eprintln!(
                "run_remote_cmd error: {x}. Try 'ssh {username}@{hostname}' and run '{remote_cmd}'"
            );
            process::exit(2);
        }
        x if x > 0 => {
            eprintln!("Remote helper exit code {x}");
            process::exit(x);
        }
        _ => unreachable!(),
    }

    let first_line = output
        .lines()
        .next()
        .expect("No response from remote or empty directory");
    if !first_line.contains(HELPER_SEP) {
        eprintln!("Remote helper error: {output}");
        process::exit(2);
    }

    let remote: HashMap<&str, u32> = output
        .lines()
        .map(|l| {
            l.split_once(HELPER_SEP)
                .map_or(("", 0), |(k, v)| (k, v.parse().unwrap()))
        })
        .filter(|(name, _)| is_include_hidden || !name.starts_with('.'))
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
    if verbose {
        println!("Comparing local and remote files");
    }
    let mut num_upload_bytes = 0;
    let mut upload = Vec::new();
    for (filename, (l_crc32, l_size)) in local.iter() {
        match remote.get(filename.as_str()) {
            None => {
                upload.push(filename);
                num_upload_bytes += l_size;
            }
            Some(r_crc32) if r_crc32 != l_crc32 => {
                upload.push(filename);
                num_upload_bytes += l_size;
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
        ssh = ssh.switch_to_dry_run();
    } else {
        let num_upload_files = upload.len();
        thread::spawn(move || run_output(num_upload_files, num_upload_bytes, progress_receiver));
    }

    if upload.is_empty() && delete.is_empty() {
        println!("Directories are already identical");
        ssh.stop();
        return Ok(());
    }

    // action

    let t_start = Instant::now();
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
            true,
        )?;
    }

    if verbose {
        println!("Delete remote files that are absent locally");
    }
    for filename in delete {
        ssh.delete(&format!("{dst_dir}{}", filename))?;
    }

    ssh.stop();
    let took_s = t_start.elapsed();
    thread::sleep(Duration::from_millis(10)); // make sure Finished is last msg
    let _ = progress_sender.send(Progress::Finished(took_s));

    Ok(())
}

// returns map of filepath->(checksum, filesize)
fn checksum_dir(
    path: path::PathBuf,
    is_include_hidden: bool,
) -> Result<HashMap<String, (u32, u64)>, anyhow::Error> {
    let path_len = path.to_string_lossy().len();
    let mut out = HashMap::with_capacity(64);
    let mut dirs = vec![path];

    while !dirs.is_empty() {
        let next_dir = dirs.pop().unwrap();
        for entry in fs::read_dir(next_dir)? {
            let file = entry?;
            let filename = file.file_name().to_string_lossy().into_owned();
            if filename.starts_with('.') && !is_include_hidden {
                continue;
            }
            let file_type = file.file_type()?;
            if file_type.is_dir() {
                dirs.push(file.path());
            } else {
                let f = fs::File::open(file.path())?;
                let file_size = f.metadata()?.len();
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
                    ((checksum & CRC32) as u32, file_size),
                );
            }
        }
    }
    Ok(out)
}

/* TODO
 - rcpl-h (asm) if file > size crc in a thread (with max thread as num CPUs)
 - rcpl-h (asm) make it as small as possible
 - embed the helper
*/
