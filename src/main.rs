use core::arch::x86_64::_mm_crc32_u64;
use std::env;
use std::fs;
use std::io::{BufReader, Read};
use std::path;
use std::process;

const USAGE: &str = r#"USAGE: rcple src_dir remote_dst_dir
       Example: rcple /home/graham/myfiles graham@myhost.com:/var/www/myfiles"#;

const CRC32: u64 = 0xFFFFFFFF;

fn main() {
    let mut args = env::args();
    if args.len() != 3 {
        eprintln!("{}", USAGE);
        process::exit(1);
    }

    args.next(); // skip program name
    let src_dir = args.next().unwrap();

    // TODO: work on the assembly version
    // then scp it here
    // then ssh run it, parse it's stdout as 'remote' values

    let local = match checksum_dir(src_dir.clone().into()) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("Error on local dir {}: {}", src_dir, err);
            process::exit(1);
        },
    };
    println!("{:?}", local);

    // now that we both local and remote compare and upload the differences
    // catching new files, and deleting removed files
}

fn checksum_dir(path: path::PathBuf) -> Result<Vec<(String, u32)>, anyhow::Error> {
    let mut out = Vec::new();
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
                            checksum = unsafe {
                                _mm_crc32_u64(checksum, u64::from_le_bytes(ubuf))
                            };
                        }
                        Err(err) => {
                            return Err(err.into());
                        }
                    }
                }
                out.push((
                    file.path().to_string_lossy().into_owned(),
                    (checksum & CRC32) as u32,
                ));
            }
        }
    }
    Ok(out)
}
