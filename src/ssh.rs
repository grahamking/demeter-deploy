#![allow(dead_code)]
#![allow(non_camel_case_types)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};

use anyhow::anyhow;

// TODO: for all the SSHResult returns, if ERROR call ssh_get_error like on ssh_connect
// for sftp maybe call sftp_get_error ?

//
// Public API
// Start with: SSH::new
//

pub struct SSH {
    session: *mut c_void,
}

impl SSH {

    // libssh version
    pub fn version() -> String {
        unsafe { CStr::from_ptr(ssh_version(0)) }.to_str().unwrap().to_string()
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

        Ok(SSH { session })
    }

    pub fn run_remote_cmd(&self, cmd: &str) -> Result<String, anyhow::Error> {
        let channel = unsafe { ssh_channel_new(self.session) };
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

    pub fn sftp(&self) -> Result<SFTP, anyhow::Error> {
        let sftp_session = unsafe { sftp_new(self.session) };
        let sftp_init_ret = unsafe { sftp_init(sftp_session) };
        if matches!(sftp_init_ret, SSHResult::ERROR) {
            let err_msg = unsafe { CStr::from_ptr(ssh_get_error(self.session)) };
            return Err(anyhow!("SFTP init ERR: {}", err_msg.to_string_lossy()));
        }
        Ok(SFTP {
            session: sftp_session,
        })
    }

    /*
    pub fn upload(&self, filename: &str, from_dir: &str, to_dir: &str) -> Result<(), anyhow::Error> {
        let sftp = ssh.sftp()?;
        let sfile = sftp.open("/tmp/rcple-h", O_WRONLY | O_CREAT | O_TRUNC, 0o700);
        let bytes_written = sfile.write(&helper_bytes);
        if bytes_written != helper_bytes.len() {
            eprintln!("Short write: {bytes_written} / {}", helper_bytes.len());
        }
        Ok(())
    }
    */
}

impl Drop for SSH {
    fn drop(&mut self) {
        unsafe {
            ssh_disconnect(self.session);
            ssh_free(self.session);
        }
    }
}

pub struct SFTP {
    session: SFTPSession,
}

impl SFTP {
    pub fn open(&self, filename: &str, mode: i32, perms: i32) -> SFTPFile {
        let remote_filename = CString::new(filename).unwrap();
        let handle = unsafe { sftp_open(self.session, remote_filename.as_ptr(), mode, perms) };
        SFTPFile { handle }
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
    pub fn write(&self, data: &[u8]) -> usize {
        unsafe { sftp_write(self.handle, data.as_ptr(), data.len() as i32) as usize }
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

#[derive(Debug)]
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
    fn ssh_version(min: c_int) -> *const c_char;
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
    fn ssh_channel_read(c: SSHChannel, dest: *mut u8, count: u32, is_stderr: c_int) -> c_int;
    fn ssh_channel_send_eof(c: SSHChannel);
    fn ssh_channel_close(c: SSHChannel);

    fn sftp_new(s: SSHSession) -> SFTPSession;
    fn sftp_free(sftp: SFTPSession);
    fn sftp_init(sftp: SFTPSession) -> SSHResult;
    fn sftp_get_error(sftp: SFTPSession) -> SFTPError;

    fn sftp_open(
        sftp: SFTPSession,
        file: *const c_char,
        accesstype: c_int,
        mode: c_int,
    ) -> SFTPFileHandle;
    fn sftp_write(sfile: SFTPFileHandle, buf: *const u8, count: c_int) -> c_int;
    fn sftp_close(sfile: SFTPFileHandle) -> SSHResult;

    //fn ssh_userauth_publickey(
    //    s: SSHSession,
    //    username: *const c_char,
    //    privkey: SSHKey,
    //) -> c_int;
    //fn ssh_get_log_level() -> libc::c_int;
}
