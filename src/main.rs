use core::arch::x86_64::_mm_crc32_u64;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{BufReader, Read};
use std::os::raw::c_int;
use std::path;
use std::process;

use anyhow;

mod ssh;
use ssh::SSH;

const USAGE: &str = r#"USAGE: rcple src_dir remote_dst_dir
       Example: rcple /home/graham/myfiles graham@myhost.com:/var/www/myfiles"#;

const CRC32: u64 = 0xFFFFFFFF;
const HELPER: &str = "/home/graham/src/rcple/asm/rcple-h";

// These are in libc crate, but no dependencies is nice
const O_WRONLY: c_int = 1;
const O_CREAT: c_int = 0o100;
const O_TRUNC: c_int = 0o1000;

fn main() -> Result<(), anyhow::Error> {
    let mut args = env::args();
    if args.len() != 3 {
        eprintln!("{}", USAGE);
        process::exit(1);
    }

    args.next(); // skip program name
    let mut src_dir = args.next().unwrap();
    if !src_dir.ends_with("/") {
        src_dir.push('/');
    }
    let dst_dir = args.next().unwrap();

    // check we have the helper binary
    let helper_bytes = match fs::read(HELPER) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("Missing helper binary. open '{}': {}.", HELPER, err);
            return Ok(());
        }
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

    let ssh = SSH::new("localhost", "graham", ssh::LogLevel::WARNING)?;

    let sftp = ssh.sftp()?;
    let sfile = sftp.open("/tmp/rcple-h", O_WRONLY | O_CREAT | O_TRUNC, 0o700);
    let bytes_written = sfile.write(&helper_bytes);
    if bytes_written != helper_bytes.len() {
        eprintln!("Short write: {bytes_written} / {}", helper_bytes.len());
        return Ok(());
    }
    drop(sfile);
    drop(sftp);

    let output = ssh.run_remote_cmd(&format!("/tmp/rcple-h {}", dst_dir))?;

    let remote: HashMap<&str, u32> = output
        .lines()
        .map(|l| {
            l.split_once(':')
                .map_or(("", 0), |(k, v)| (k, v.parse().unwrap()))
        })
        .collect();

    // compare

    for (filename, ref l_crc32) in local {
        let r_filename = filename.replacen(&src_dir, "./", 1);
        match remote.get(r_filename.as_str()) {
            None => {
                println!("Upload new: {}", filename);
            },
            Some(r_crc32) if r_crc32 != l_crc32 => {
                println!("Upload changed: {}", filename);
            },
            _ => {}, // they are the same
        }
    }

    // now that we both local and remote compare and upload the differences
    // catching new files, and deleting removed files
    // TODO add a --dry-run flag

    Ok(())
}

fn checksum_dir(path: path::PathBuf) -> Result<HashMap<String, u32>, anyhow::Error> {
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
                    file.path().to_string_lossy().into_owned(),
                    (checksum & CRC32) as u32,
                );
            }
        }
    }
    Ok(out)
}

/* TODO
 - delete (file remote but not local)
 - flag to skip dot (hidden) files (or to include them)
 - local and remote each in a thread
 - rcpl-h (asm) if file > size crc in a thread (with max thread as num CPUs)
*/
