use std::time::Duration;

pub enum Progress {
    // uploaded a file block. value should be bytes we uploaded in this block (e.g. 64).
    Part(usize),
    // uploaded a whole file. values are filepath and file size.
    Complete(String, usize),
    // uploaded all the files. value is how long the whole upload took.
    Finished(Duration),
}
