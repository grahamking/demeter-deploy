#![allow(dead_code)]
#![allow(non_camel_case_types)]
#![allow(clippy::upper_case_acronyms)]

use std::ffi::{CStr, CString};
use std::fs;
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::os::unix::fs::PermissionsExt;

use anyhow::anyhow;

// These are in libc crate, but no dependencies is nice
const O_WRONLY: c_uint = 1;
const O_CREAT: c_uint = 0o100;
const O_TRUNC: c_uint = 0o1000;

// Give libssh data in chunks of 128 KiB. I think an sftp packet is 32 KiB.
// Has to be under 256 KiB or things start to break.
const SFTP_CHUNK_SIZE: usize = 128 * 1024;

// TODO: for all the SSHResult returns, if ERROR call ssh_get_error like on ssh_connect
// For sftp maybe (also?) call sftp_get_error

//
// Public API
// Start with: SSH::new
//

pub trait Remote {
    fn run_remote_cmd(&self, cmd: &str) -> Result<String, anyhow::Error>;
    fn mkdir(&self, dir: &str, perms: u32) -> Result<(), anyhow::Error>;
    fn upload(&self, src: &str, dst: &str) -> Result<(), anyhow::Error>;
    fn delete(&self, path: &str) -> Result<(), anyhow::Error>;
}

pub struct SSH {
    sftp_session: SFTP, // comes first because must be dropped before 'session'
    session: SSHSessionWrap,
}

impl SSH {
    // libssh version
    pub fn version() -> String {
        unsafe { CStr::from_ptr(ssh_version(0)) }
            .to_str()
            .unwrap()
            .to_string()
    }

    // connect and authenticate
    pub fn new(host: &str, username: &str, log_level: LogLevel) -> Result<SSH, anyhow::Error> {
        let host = CString::new(host).unwrap();
        let username = CString::new(username).unwrap();

        unsafe { ssh_set_log_level(log_level) };
        let session = unsafe { ssh_new() };
        if session.is_null() {
            return Err(anyhow!("ssh_new retuned null"));
        }
        unsafe {
            ssh_options_set(session, SSHOption::HOST, host.as_ptr() as *const c_void);
            ssh_options_set(session, SSHOption::PORT, &22 as *const _ as _);
        }
        let connect_ret = unsafe { ssh_connect(session) };
        if matches!(connect_ret, SSHResult::ERROR) {
            let err_msg = unsafe { CStr::from_ptr(ssh_get_error(session)) };
            return Err(anyhow!("Connect ERR: {}", err_msg.to_string_lossy()));
        }

        let is_know = unsafe { ssh_session_is_known_server(session) };
        if !matches!(is_know, SSHKnownHostsResult::HOSTS_OK) {
            return Err(anyhow!(
                "Unknown host: {is_know:?}. ssh to it manually first to accept key"
            ));
        }

        unsafe {
            let auth_ret = ssh_userauth_agent(session, username.as_ptr());
            if !matches!(auth_ret, SSHAuthResult::SUCCESS) {
                return Err(anyhow!(
                    "auth err or incomplete: {auth_ret:?}. Is ssh-agent running?"
                ));
            }
        };

        let sftp_session = SSH::create_sftp(session)?;
        Ok(SSH {
            session: SSHSessionWrap(session),
            sftp_session,
        })
    }

    fn create_sftp(session: *mut c_void) -> Result<SFTP, anyhow::Error> {
        let sftp_session = unsafe { sftp_new(session) };
        let sftp_init_ret = unsafe { sftp_init(sftp_session) };
        if matches!(sftp_init_ret, SSHResult::ERROR) {
            let err_msg = unsafe { CStr::from_ptr(ssh_get_error(session)) };
            return Err(anyhow!("SFTP init ERR: {}", err_msg.to_string_lossy()));
        }
        Ok(SFTP {
            session: sftp_session,
        })
    }

    pub fn sftp(&mut self) -> &SFTP {
        &self.sftp_session
    }

    fn get_sftp_err(&self, msg: &str) -> anyhow::Error {
        let ssh_err_msg = unsafe { CStr::from_ptr(ssh_get_error(self.session.0)) };
        let sftp_err_num = unsafe { sftp_get_error(self.sftp_session.session) };
        anyhow!(
            "{}: {}. SFTP err num: {:?}.",
            msg,
            ssh_err_msg.to_string_lossy(),
            sftp_err_num
        )
    }
}

impl Remote for SSH {
    fn run_remote_cmd(&self, cmd: &str) -> Result<String, anyhow::Error> {
        let channel = unsafe { ssh_channel_new(self.session.0) };
        if channel.is_null() {
            return Err(anyhow!("channel is null"));
        }
        let ses_ret = unsafe { ssh_channel_open_session(channel) };
        if matches!(ses_ret, SSHResult::ERROR) {
            return Err(anyhow!(
                "ssh_channel_open_session err - increase log level and re-run"
            ));
        }
        let ls_command = CString::new(cmd).unwrap();
        let rc = unsafe { ssh_channel_request_exec(channel, ls_command.as_ptr()) };
        if matches!(rc, SSHResult::ERROR) {
            return Err(anyhow!(
                "ssh_channel_request_exec err - increase log level and re-un"
            ));
        }

        let mut output = String::new();
        let mut buffer = Vec::with_capacity(SSH_CMD_BUF_SIZE);
        let mut nbytes =
            unsafe { ssh_channel_read(channel, buffer.as_mut_ptr(), buffer.capacity() as u32, 0) };
        while nbytes > 0 {
            unsafe { buffer.set_len(nbytes as usize) };
            output += &String::from_utf8_lossy(&buffer);

            buffer.clear();
            nbytes = unsafe {
                ssh_channel_read(channel, buffer.as_mut_ptr(), buffer.capacity() as u32, 0)
            };
        }

        unsafe {
            ssh_channel_send_eof(channel);
            ssh_channel_close(channel);
            ssh_channel_free(channel);
        }

        Ok(output)
    }

    // Upload a local file to remote
    //
    // src: local full path of filename to upload
    // dst: remote full path of destination file to create or overwrite
    fn upload(&self, src: &str, dst: &str) -> Result<(), anyhow::Error> {
        let data = fs::read(src)?; // todo: read in chunks of SFTP_CHUNK_SIZE
        let perms = fs::metadata(src)?.permissions().mode();

        let sfile = self
            .sftp_session
            .open(dst, O_WRONLY | O_CREAT | O_TRUNC, perms)?;
        //println!("upload {} -> {}", src, dst);

        for chunk in data.chunks(SFTP_CHUNK_SIZE) {
            let ret = sfile.write(chunk);
            if ret < 0 {
                return Err(self.get_sftp_err(&format!("upload to {}", dst)));
            }
            let bytes_written = ret as usize;
            if bytes_written != chunk.len() {
                return Err(anyhow::anyhow!(
                    "Short write: {bytes_written} / {}",
                    chunk.len()
                ));
            }
        }
        Ok(())
    }

    // make remote directory
    fn mkdir(&self, dir: &str, perms: u32) -> Result<(), anyhow::Error> {
        let c_dir = CString::new(dir)?;
        let ret = unsafe { sftp_mkdir(self.sftp_session.session, c_dir.as_ptr(), perms) };
        if matches!(ret, SSHResult::ERROR) {
            let sftp_err_num = unsafe { sftp_get_error(self.sftp_session.session) };
            if sftp_err_num != SFTPError::SSH_FX_FILE_ALREADY_EXISTS {
                let ssh_err_msg = unsafe { CStr::from_ptr(ssh_get_error(self.session.0)) };
                return Err(anyhow!(
                    "mkdir: {}. SFTP err num: {:?}.",
                    ssh_err_msg.to_string_lossy(),
                    sftp_err_num
                ));
            }
        }
        Ok(())
    }

    // delete a remote file
    fn delete(&self, path: &str) -> Result<(), anyhow::Error> {
        let c_path = CString::new(path)?;
        let ret = unsafe { sftp_unlink(self.sftp_session.session, c_path.as_ptr()) };
        if matches!(ret, SSHResult::ERROR) {
            return Err(self.get_sftp_err(&format!("delete {}", path)));
        }
        Ok(())
    }
}

// Wrap the pointer so we can implement Drop
struct SSHSessionWrap(*mut c_void);

impl Drop for SSHSessionWrap {
    fn drop(&mut self) {
        unsafe {
            ssh_disconnect(self.0);
            ssh_free(self.0);
        }
    }
}

pub struct SFTP {
    session: SFTPSession,
}

impl SFTP {
    pub fn open(&self, filename: &str, mode: u32, perms: u32) -> Result<SFTPFile, anyhow::Error> {
        let remote_filename = CString::new(filename).unwrap();
        let handle = unsafe { sftp_open(self.session, remote_filename.as_ptr(), mode, perms) };
        if handle.is_null() {
            return Err(anyhow::anyhow!("sftp_open remote {filename}"));
        }
        Ok(SFTPFile { handle })
    }
}

impl Drop for SFTP {
    fn drop(&mut self) {
        unsafe {
            sftp_free(self.session);
        }
    }
}

pub struct SFTPFile {
    handle: SFTPFileHandle,
}

impl SFTPFile {
    pub fn write(&self, data: &[u8]) -> i32 {
        //println!("SFTPFile.write {} bytes", data.len() as u32);
        unsafe { sftp_write(self.handle, data.as_ptr(), data.len() as u32) }
    }
}

impl Drop for SFTPFile {
    fn drop(&mut self) {
        let sftp_close_ret = unsafe { sftp_close(self.handle) };
        if matches!(sftp_close_ret, SSHResult::ERROR) {
            eprintln!("sftp_close err");
        }
    }
}

#[repr(i32)]
pub enum LogLevel {
    NOLOG = 0, // No logging at all
    WARNING,   // Only warnings
    PROTOCOL,  // High level protocol information
    PACKET,    // Lower level protocol infomations, packet level
    FUNCTIONS, // Every function path
}

pub struct MockSSH {}

impl Remote for MockSSH {
    fn run_remote_cmd(&self, cmd: &str) -> Result<String, anyhow::Error> {
        println!("would run cmd '{cmd}'");
        Ok("".to_string())
    }
    fn mkdir(&self, dir: &str, perms: u32) -> Result<(), anyhow::Error> {
        println!("would mkdir {dir} with perms {perms:o}");
        Ok(())
    }
    fn upload(&self, src: &str, dst: &str) -> Result<(), anyhow::Error> {
        println!("would upload {src} -> {dst}");
        Ok(())
    }
    fn delete(&self, path: &str) -> Result<(), anyhow::Error> {
        println!("would delete {path}");
        Ok(())
    }
}

//
// Internal
//

type SSHSession = *mut c_void;
type SSHChannel = *mut c_void;
type SFTPSession = *mut c_void;
type SFTPFileHandle = *mut c_void;
//type SSHKey = *const u8;

const SSH_CMD_BUF_SIZE: usize = 1024;

#[repr(u32)]
enum SSHOption {
    HOST = 0,
    PORT,
    PORT_STR,
    FD,
    USER,
    SSH_DIR,
    IDENTITY,
    ADD_IDENTITY,
    KNOWNHOSTS,
}

#[repr(i32)]
enum SSHResult {
    ERROR = -1,
    OK = 0,
}

#[derive(Debug)]
#[repr(i32)]
enum SSHKnownHostsResult {
    // There had been an error checking the host.
    HOSTS_ERROR = -2,

    // The known host file does not exist. The host is thus unknown. File will
    // be created if host key is accepted.
    SSH_KNOWN_HOSTS_NOT_FOUND = -1,

    // The server is unknown. User should confirm the public key hash is correct.
    HOSTS_UNKNOWN = 0,

    // The server is known and has not changed.
    HOSTS_OK = 1,

    // The server key has changed. Either you are under attack or the
    // administrator changed the key. You HAVE to warn the user about a
    // possible attack.
    HOSTS_CHANGED = 2,

    // The server gave use a key of a type while we had an other type recorded.
    // It is a possible attack.
    HOSTS_OTHER = 3,
}

#[derive(Debug, PartialEq)]
#[repr(u32)]
enum SFTPError {
    /** No error */
    SSH_FX_OK = 0,
    /** End-of-file encountered */
    SSH_FX_EOF = 1,
    /** File doesn't exist */
    SSH_FX_NO_SUCH_FILE = 2,
    /** Permission denied */
    SSH_FX_PERMISSION_DENIED = 3,
    /** Generic failure */
    SSH_FX_FAILURE = 4,
    /** Garbage received from server */
    SSH_FX_BAD_MESSAGE = 5,
    /** No connection has been set up */
    SSH_FX_NO_CONNECTION = 6,
    /** There was a connection, but we lost it */
    SSH_FX_CONNECTION_LOST = 7,
    /** Operation not supported by the server */
    SSH_FX_OP_UNSUPPORTED = 8,
    /** Invalid file handle */
    SSH_FX_INVALID_HANDLE = 9,
    /** No such file or directory path exists */
    SSH_FX_NO_SUCH_PATH = 10,
    /** An attempt to create an already existing file or directory has been made */
    SSH_FX_FILE_ALREADY_EXISTS = 11,
    /** We are trying to write on a write-protected filesystem */
    SSH_FX_WRITE_PROTECT = 12,
    /** No media in remote drive */
    SSH_FX_NO_MEDIA = 13,
}

#[derive(Debug)]
#[repr(i32)]
enum SSHAuthResult {
    SUCCESS = 0,
    DENIED,
    PARTIAL,
    INFO,
    AGAIN,
    ERROR = -1,
}

//
// FFI
// Wrap libssh. The below is from /usr/include/libssh/libssh.h and sftp.h

#[link(name = "ssh")]
extern "C" {
    fn ssh_version(min: c_uint) -> *const c_char;
    fn ssh_set_log_level(level: LogLevel) -> c_int;
    fn ssh_options_set(s: SSHSession, opt_type: SSHOption, value: *const c_void) -> c_int;

    fn ssh_new() -> SSHSession;
    fn ssh_free(s: SSHSession);

    fn ssh_connect(s: SSHSession) -> SSHResult;
    fn ssh_disconnect(s: SSHSession);

    fn ssh_get_error(s: SSHSession) -> *const c_char;
    fn ssh_session_is_known_server(s: SSHSession) -> SSHKnownHostsResult;

    fn ssh_userauth_agent(s: SSHSession, username: *const c_char) -> SSHAuthResult;

    fn ssh_channel_new(s: SSHSession) -> SSHChannel;
    fn ssh_channel_free(c: SSHChannel);
    fn ssh_channel_open_session(c: SSHChannel) -> SSHResult;
    fn ssh_channel_request_exec(c: SSHChannel, cmd: *const c_char) -> SSHResult;
    fn ssh_channel_read(c: SSHChannel, dest: *mut u8, count: u32, is_stderr: c_uint) -> c_int;
    fn ssh_channel_send_eof(c: SSHChannel);
    fn ssh_channel_close(c: SSHChannel);

    fn sftp_new(s: SSHSession) -> SFTPSession;
    fn sftp_free(sftp: SFTPSession);
    fn sftp_init(sftp: SFTPSession) -> SSHResult;
    fn sftp_get_error(sftp: SFTPSession) -> SFTPError;
    fn sftp_mkdir(sftp: SFTPSession, dir: *const c_char, perms: c_uint) -> SSHResult;
    fn sftp_unlink(sftp: SFTPSession, path: *const c_char) -> SSHResult;

    fn sftp_open(
        sftp: SFTPSession,
        file: *const c_char,
        accesstype: c_uint,
        mode: c_uint,
    ) -> SFTPFileHandle;
    fn sftp_write(sfile: SFTPFileHandle, buf: *const u8, count: c_uint) -> i32;
    fn sftp_close(sfile: SFTPFileHandle) -> SSHResult;

    //fn ssh_userauth_publickey(
    //    s: SSHSession,
    //    username: *const c_char,
    //    privkey: SSHKey,
    //) -> c_int;
    //fn ssh_get_log_level() -> libc::c_int;
}
