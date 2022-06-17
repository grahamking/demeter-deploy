use std::cmp::max;
use std::io::{stdout, Write};

use crate::progress_message::Progress;
use crossbeam_channel::Receiver;

const KB: f64 = 1024.0;
const MB: f64 = KB * 1024.0;

// Display thread
pub fn run_output(total_files: usize, total_bytes: u64, recv: Receiver<Progress>) {
    if total_files == 0 || total_bytes == 0 {
        return;
    }
    use Progress::*;
    let mut files_so_far = 0;
    let mut bytes_so_far = 0;
    while let Ok(progress) = recv.recv() {
        match progress {
            Complete(filename, size) => {
                let msg = format!("\rUploaded {filename} ({})", humanize(size));
                {
                    let mut out = stdout();
                    out.write_all(msg.as_bytes()).unwrap();
                    out.write_all(b"\x1B[K\x0A").unwrap(); // clear to end of line + \n
                    out.flush().unwrap();
                }
                files_so_far += 1;
            }
            Part(bytes) => {
                bytes_so_far += bytes;
                let bytes_pct = (bytes_so_far as f64) / (total_bytes as f64) * 100.0;
                let msg = format!(
                    "\rProgress: {files_so_far} / {total_files} files, {bytes_pct:.0}% of bytes."
                );
                {
                    let mut out = stdout();
                    out.write_all(msg.as_bytes()).unwrap();
                    out.flush().unwrap();
                }
            }
            Finished(elapsed) => {
                let bytes_sec = total_bytes / max(elapsed.as_secs(), 1);
                let msg = format!(
                    "\rFinished {total_files} files, {} in {:?} ({bytes_sec} KiB/s)\n",
                    humanize(total_bytes as usize),
                    elapsed,
                );
                let mut out = stdout();
                out.write_all(msg.as_bytes()).unwrap();
                out.flush().unwrap();
            }
        }
    }
}

fn humanize(size: usize) -> String {
    let fsize = size as f64;
    if fsize > MB {
        format!("{:02} MiB", (fsize / MB).round())
    } else if fsize > KB {
        format!("{} KiB", (fsize / KB).round() as usize)
    } else {
        format!("{size} bytes")
    }
}
