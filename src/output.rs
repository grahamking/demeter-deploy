use std::cmp::max;
use std::collections::HashMap;
use std::io::{stdout, Write};
use std::time::Duration;

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
    let mut in_progress = HashMap::new();
    let mut files_so_far = 0;
    let mut bytes_so_far = 0;
    let mut prev_bytes_pct = 0.0;
    while let Ok(progress) = recv.recv() {
        match progress {
            Start(filename, size) => {
                in_progress.insert(filename, size);
            }
            Complete(filename) => {
                let size = in_progress
                    .remove(&filename)
                    .expect("Got Complete for file that didn't have Start");
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
                if (prev_bytes_pct - bytes_pct).abs() < 1.0 {
                    // only update display if > 1% progress
                    continue;
                }
                prev_bytes_pct = bytes_pct;
                let mut active_files: Vec<&str> = in_progress.keys().map(|s| s.as_ref()).collect();
                active_files.sort();
                let msg = format!(
                    "\rProgress: {} / {} files, {:.0}% of bytes. [{}]",
                    files_so_far,
                    total_files,
                    bytes_pct,
                    active_files.join(" "),
                );
                {
                    let mut out = stdout();
                    out.write_all(msg.as_bytes()).unwrap();
                    out.flush().unwrap();
                }
            }
            Finished(elapsed) => {
                let msg = format!(
                    "\rFinished {total_files} files, {} at {}.\n",
                    humanize(total_bytes),
                    humanize_speed(total_bytes, elapsed),
                );
                let mut out = stdout();
                out.write_all(msg.as_bytes()).unwrap();
                out.flush().unwrap();
            }
        }
    }
}

fn humanize(size: u64) -> String {
    let fsize = size as f64;
    if fsize > MB {
        format!("{} MiB", (fsize / MB).round())
    } else if fsize > KB {
        format!("{} KiB", (fsize / KB).round())
    } else {
        format!("{size} bytes")
    }
}

fn humanize_speed(size: u64, time: Duration) -> String {
    let t = max(time.as_secs(), 1) as f64;
    let fsize = size as f64;
    if fsize > MB {
        format!("{:.2} MiB/s", (fsize / MB).round() / t)
    } else if fsize > KB {
        format!("{:.0} KiB/s", (fsize / KB).round() / t)
    } else {
        format!("{} B/s", fsize / t)
    }
}
