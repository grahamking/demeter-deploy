use std::time::Duration;

pub enum Progress {
    // Start uploading this filename. u64 if file size in bytes.
    Start(String, u64),
    // Uploaded a file block. value should be bytes we uploaded in this block (e.g. 64).
    Part(usize),
    // Uploaded a whole file.
    Complete(String),
    // Uploaded all the files. value is how long the whole upload took.
    Finished(Duration),
}
