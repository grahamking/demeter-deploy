use core::{ffi::c_char, mem::zeroed};

use crate::{is_ignore_dir, itoa};

#[test]
fn test_itoa() {
    let mut buf = [0u8; 8];
    unsafe { itoa(0, buf.as_mut_ptr() as *mut c_char) };
    assert_eq!("0\0".as_bytes(), &buf[0..2]);

    buf = unsafe { zeroed() };
    unsafe { itoa(1, buf.as_mut_ptr() as *mut c_char) };
    assert_eq!("1\0".as_bytes(), &buf[0..2]);

    buf = unsafe { zeroed() };
    unsafe { itoa(123, buf.as_mut_ptr() as *mut c_char) };
    assert_eq!("123\0".as_bytes(), &buf[0..4]);
}

#[test]
fn test_is_ignore_dir() {
    unsafe {
        assert!(is_ignore_dir(".\0".as_ptr() as *const i8));
        assert!(is_ignore_dir("..\0".as_ptr() as *const i8));
        assert!(!is_ignore_dir("bin\0".as_ptr() as *const i8));
    }
}
