use core::arch::x86_64::_mm_crc32_u64;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{BufReader, Read};
use std::path;
use std::process;

mod ssh;
use ssh::SSH;

const USAGE: &str = r#"USAGE: rcple src_dir remote_dst_dir
       Example: rcple /home/graham/myfiles graham@myhost.com:/var/www/myfiles"#;

const CRC32: u64 = 0xFFFFFFFF;
const HELPER: &str = "/home/graham/src/rcple/asm/rcple-h";

fn main() -> Result<(), anyhow::Error> {
    let verbose = true; // will become a cmd line flag

    let mut args = env::args();
    if args.len() != 3 {
        eprintln!("{}", USAGE);
        process::exit(1);
    }

    args.next(); // skip program name
    let mut src_dir = args.next().unwrap();
    if !src_dir.ends_with('/') {
        src_dir.push('/');
    }
    let mut dst_dir = args.next().unwrap();
    if !dst_dir.ends_with('/') {
        dst_dir.push('/');
    }

    // check we have the helper binary
    if !path::Path::new(HELPER).exists() {
        eprintln!("Helper binary '{}' not found.", HELPER);
        return Ok(());
    };

    // local

    let local = match checksum_dir(src_dir.clone().into()) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("Error on local dir {}: {}", src_dir, err);
            process::exit(1);
        }
    };

    // remote

    if verbose {
        println!("Using libssh {}", SSH::version());
    }
    let ssh = SSH::new("localhost", "graham", ssh::LogLevel::WARNING)?;
    ssh.upload(HELPER, "/tmp/rcple-h")?;

    let output = ssh.run_remote_cmd(&format!("/tmp/rcple-h {}", dst_dir))?;

    let remote: HashMap<&str, u32> = output
        .lines()
        .map(|l| {
            l.split_once(':')
                .map_or(("", 0), |(k, v)| (k, v.parse().unwrap()))
        })
        .collect();

    // compare

    let mut upload = Vec::new();
    for (filename, l_crc32) in local.iter() {
        match remote.get(filename.as_str()) {
            None => {
                if verbose {
                    println!("Upload new: {}", filename);
                }
                upload.push(filename);
            }
            Some(r_crc32) if r_crc32 != l_crc32 => {
                if verbose {
                    println!("Upload changed: {}", filename);
                }
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
            if verbose {
                println!("Delete remote: {}", filename);
            }
            delete.push(filename);
        }
    }

    // action

    for filename in upload {
        // Do we need to make the parent dir(s)?
        let p = path::PathBuf::from(filename);
        if let Some(dir) = p.parent() {
            if !remote_dirs.contains(dir) {
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
        // TODO: work here
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
 - flag for dry-run
 - flag to skip dot (hidden) files (or to include them)
 - adding missing directories - sftp_mkdir
    maybe put in output as <dir_path>:DIR? then we know it's an add or remove
 - local and remote each in a thread
 - rcpl-h (asm) if file > size crc in a thread (with max thread as num CPUs)
*/
