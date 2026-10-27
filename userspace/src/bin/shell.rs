//! Interactive shell (user-space image).
//!
//! Reads keystrokes from the keyboard server via IPC and implements a small
//! command interpreter. File commands (`ls`, `cat`, `run`) talk to the
//! storage server over the `fs_proto` protocol; job control (`kill`, `stop`,
//! `cont`) uses the KILL syscall; `ps` reports the well-known servers.

#![no_std]
#![no_main]

use userspace::{cmd, exec, fs_proto, get_server_pid, heap, ipc_recv_timeout, ipc_send, kill, msg, sig, syscall0, syscall_nr, vga_clear, vga_write, IpcMessage};

const LINE_BUF_SIZE: usize = 128;

/// Ticks (100 Hz) the shell waits for a key event before looping again.
const KEY_WAIT_TICKS: usize = 20;
/// Ticks (100 Hz) the shell waits for a reply from the storage server.
const FS_WAIT_TICKS: usize = 200;

/// Number of received-but-not-yet-consumed messages the shell can hold.
///
/// The shell multiplexes two conversations on a single IPC queue: key events
/// from the keyboard server and replies from the storage server. A message
/// that arrives while we are waiting for a *different* type is parked here so
/// it is neither lost nor mistaken for the wrong reply.
const INBOX_CAPACITY: usize = 16;

/// Parked messages, used as a small FIFO ring.
struct Inbox {
    msgs: [Option<IpcMessage>; INBOX_CAPACITY],
    head: usize,
    tail: usize,
}

impl Inbox {
    const fn new() -> Self {
        Self {
            msgs: [None; INBOX_CAPACITY],
            head: 0,
            tail: 0,
        }
    }

    /// Park a message. Returns false when the inbox is full (message dropped).
    fn push(&mut self, m: IpcMessage) -> bool {
        let next = (self.tail + 1) % INBOX_CAPACITY;
        if next == self.head {
            return false;
        }
        self.msgs[self.tail] = Some(m);
        self.tail = next;
        true
    }

    /// Take the oldest parked message.
    fn pop(&mut self) -> Option<IpcMessage> {
        if self.head == self.tail {
            return None;
        }
        let m = self.msgs[self.head].take();
        self.head = (self.head + 1) % INBOX_CAPACITY;
        m
    }
}

fn print_char(ch: u8) {
    unsafe { let _ = vga_write(ch as usize); }
}

fn print_str(s: &str) {
    for &b in s.as_bytes() {
        print_char(b);
    }
}

fn print_u32(mut n: u32) {
    if n == 0 {
        print_char(b'0');
        return;
    }
    let mut digits = [0u8; 10];
    let mut i = digits.len();
    while n > 0 {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    for &b in digits[i..].iter() {
        print_char(b);
    }
}

/// Receive the next message whose type is one of `wanted`.
///
/// Messages of other types are parked in `inbox` and returned later, so the
/// shell never loses a key event while it is waiting for a storage reply.
/// Returns `None` when the wait times out.
fn recv_matching(inbox: &mut Inbox, wanted: &[u16], timeout_ticks: usize) -> Option<IpcMessage> {
    // First, look at what was parked earlier.
    let mut parked: [Option<IpcMessage>; INBOX_CAPACITY] = [None; INBOX_CAPACITY];
    let mut parked_len = 0usize;
    while let Some(m) = inbox.pop() {
        if wanted.contains(&m.msg_type) {
            // Put back everything we did not need, in order.
            for i in (0..parked_len).rev() {
                inbox.push(parked[i].unwrap());
            }
            return Some(m);
        }
        parked[parked_len] = Some(m);
        parked_len += 1;
    }

    let mut buf: IpcMessage = unsafe { core::mem::zeroed() };
    loop {
        let ret = unsafe { ipc_recv_timeout(&mut buf as *mut IpcMessage as usize, timeout_ticks) };
        if ret <= 0 {
            // Timeout (or error): nothing matched.
            for i in (0..parked_len).rev() {
                inbox.push(parked[i].unwrap());
            }
            return None;
        }
        if wanted.contains(&buf.msg_type) {
            for i in (0..parked_len).rev() {
                inbox.push(parked[i].unwrap());
            }
            return Some(buf);
        }
        // Not what we are waiting for: park it and keep waiting.
        if !inbox.push(buf) {
            for i in (0..parked_len).rev() {
                inbox.push(parked[i].unwrap());
            }
            return None;
        }
    }
}

/// Ask the keyboard server for the next buffered key.
///
/// Protocol:
///   1. Send MSG_KEY_REQUEST (type 4) to the keyboard server.
///   2. Receive MSG_KEY_EVENT (type 5) reply.
///   3. If `reply.data[1] == 1`, `reply.data[0]` is the ASCII character.
///
/// Returns `None` if no key is available or the request failed.
fn get_key(inbox: &mut Inbox, kbd_pid: usize) -> Option<u8> {
    // Send a key request to the keyboard server.
    let req = IpcMessage::new(0, msg::KEY_REQUEST, [0u8; 60]);
    if unsafe { ipc_send(kbd_pid, &req as *const IpcMessage as usize) } < 0 {
        return None;
    }
    // Receive the reply, ignoring (and parking) anything else.
    let reply = recv_matching(inbox, &[msg::KEY_EVENT], KEY_WAIT_TICKS)?;
    if reply.data[1] == 1 {
        Some(reply.data[0])
    } else {
        None
    }
}

/// Send an IPC message, retrying while the receiver's queue is full.
/// Gives up after a bound so a dead peer cannot wedge the shell.
fn send_retry(dst: usize, msg: &IpcMessage) -> bool {
    unsafe {
        for _ in 0..10_000u32 {
            if ipc_send(dst, msg as *const IpcMessage as usize) == 0 {
                return true;
            }
            let _ = syscall0(syscall_nr::SCHEDULE);
        }
        false
    }
}

/// One root-directory entry as reported by the storage server.
#[derive(Clone, Copy)]
struct FsEntry {
    raw: [u8; 11],
    size: u32,
}

/// List the root directory into `out`. Returns the entry count, or -1.
fn fs_list(inbox: &mut Inbox, storage: usize, out: &mut [FsEntry]) -> isize {
    let req = IpcMessage::new(0, fs_proto::LIST, [0u8; 60]);
    if !send_retry(storage, &req) {
        return -1;
    }
    let mut count = 0;
    loop {
        let reply = match recv_matching(inbox, &[fs_proto::ENTRY, fs_proto::END], FS_WAIT_TICKS) {
            Some(r) => r,
            None => return -1,
        };
        match reply.msg_type {
            fs_proto::ENTRY => {
                if count < out.len() {
                    let mut raw = [0u8; 11];
                    raw.copy_from_slice(&reply.data[..11]);
                    let size = u32::from_le_bytes([reply.data[11], reply.data[12], reply.data[13], reply.data[14]]);
                    out[count] = FsEntry { raw, size };
                    count += 1;
                }
            }
            fs_proto::END => return count as isize,
            _ => {}
        }
    }
}

/// Read up to `want` bytes of `name` at `off` into `dst`. Returns the byte
/// count, or `fs_proto::READ_ERROR` on failure.
fn fs_read_chunk(inbox: &mut Inbox, storage: usize, name: &[u8; 11], off: usize, want: usize, dst: *mut u8) -> u32 {
    let mut d = [0u8; 60];
    d[..11].copy_from_slice(name);
    d[12..16].copy_from_slice(&(off as u32).to_le_bytes());
    d[16..20].copy_from_slice(&(want as u32).to_le_bytes());
    let req = IpcMessage::new(0, fs_proto::READ, d);
    if !send_retry(storage, &req) {
        return fs_proto::READ_ERROR;
    }
    let reply = match recv_matching(inbox, &[fs_proto::DATA], FS_WAIT_TICKS) {
        Some(r) => r,
        None => return fs_proto::READ_ERROR,
    };
    let n = u32::from_le_bytes([reply.data[0], reply.data[1], reply.data[2], reply.data[3]]);
    if n == fs_proto::READ_ERROR || n as usize > fs_proto::DATA_MAX {
        return fs_proto::READ_ERROR;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(reply.data[4..].as_ptr(), dst, n as usize);
    }
    n
}

/// Wait for one DATA reply (sent earlier as part of a pipelined batch) and
/// copy its payload to `dst`. Messages arrive FIFO from a single sender, so
/// the first DATA is the next chunk in order. Returns the byte count, or
/// `fs_proto::READ_ERROR` on timeout/mismatch.
fn recv_data_chunk(inbox: &mut Inbox, want: usize, dst: *mut u8) -> u32 {
    let reply = match recv_matching(inbox, &[fs_proto::DATA], FS_WAIT_TICKS) {
        Some(r) => r,
        None => return fs_proto::READ_ERROR,
    };
    let n = u32::from_le_bytes([reply.data[0], reply.data[1], reply.data[2], reply.data[3]]);
    if n == fs_proto::READ_ERROR || n as usize > fs_proto::DATA_MAX || n as usize != want {
        return fs_proto::READ_ERROR;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(reply.data[4..].as_ptr(), dst, n as usize);
    }
    n
}

/// Print a raw 8.3 name as `BASE.EXT` (or `BASE` when the extension is blank).
fn print_83(raw: &[u8; 11]) {
    let mut base_end = 8;
    while base_end > 0 && raw[base_end - 1] == b' ' {
        base_end -= 1;
    }
    for &b in raw[..base_end].iter() {
        print_char(b);
    }
    let mut ext_end = 3;
    while ext_end > 0 && raw[8 + ext_end - 1] == b' ' {
        ext_end -= 1;
    }
    if ext_end > 0 {
        print_char(b'.');
        for &b in raw[8..8 + ext_end].iter() {
            print_char(b);
        }
    }
}

fn cmd_ls(inbox: &mut Inbox, storage: usize) {
    let mut entries = [FsEntry { raw: [0u8; 11], size: 0 }; 16];
    let n = fs_list(inbox, storage, &mut entries);
    if n < 0 {
        print_str("ls: storage server did not answer\n");
        return;
    }
    for e in entries.iter().take(n as usize) {
        print_str("  ");
        print_83(&e.raw);
        print_str("  size=");
        print_u32(e.size);
        print_char(b'\n');
    }
}

fn cmd_cat(inbox: &mut Inbox, storage: usize, name_arg: &[u8]) {
    let raw = cmd::to_83(name_arg);
    let mut off = 0usize;
    loop {
        let mut chunk = [0u8; fs_proto::DATA_MAX];
        let n = fs_read_chunk(inbox, storage, &raw, off, fs_proto::DATA_MAX, chunk.as_mut_ptr());
        if n == fs_proto::READ_ERROR {
            print_str("cat: no such file\n");
            return;
        }
        if n == 0 {
            break;
        }
        for &b in chunk[..n as usize].iter() {
            print_char(b);
        }
        off += n as usize;
    }
    print_char(b'\n');
}

fn cmd_run(inbox: &mut Inbox, storage: usize, name_arg: &[u8]) {
    let raw = cmd::to_83(name_arg);
    // Find the size first (LIST), so the heap buffer is exactly file-sized.
    let mut entries = [FsEntry { raw: [0u8; 11], size: 0 }; 16];
    let n = fs_list(inbox, storage, &mut entries);
    if n < 0 {
        print_str("run: storage server did not answer\n");
        return;
    }
    let mut size = None;
    for e in entries.iter().take(n as usize) {
        if e.raw == raw {
            size = Some(e.size as usize);
            break;
        }
    }
    let size = match size {
        Some(s) => s,
        None => {
            print_str("run: no such file\n");
            return;
        }
    };
    if size == 0 || size > 64 * 1024 {
        print_str("run: bad file size\n");
        return;
    }
    let buf = unsafe { heap::alloc(size) };
    if buf == 0 {
        print_str("run: out of memory\n");
        return;
    }
    let mut off = 0usize;
    // Pipeline the chunk reads in batches: the kernel IPC queues hold 64
    // messages, so fire off a whole batch of READs, then collect the DATAs
    // in FIFO order. One round-trip per batch instead of one per chunk —
    // 61 sequential round-trips never fit a slow-CI step budget.
    while off < size {
        let batch = core::cmp::min(32, (size - off + fs_proto::DATA_MAX - 1) / fs_proto::DATA_MAX);
        for i in 0..batch {
            let o = off + i * fs_proto::DATA_MAX;
            let want = core::cmp::min(fs_proto::DATA_MAX, size - o);
            let mut d = [0u8; 60];
            d[..11].copy_from_slice(&raw);
            d[12..16].copy_from_slice(&(o as u32).to_le_bytes());
            d[16..20].copy_from_slice(&(want as u32).to_le_bytes());
            let req = IpcMessage::new(0, fs_proto::READ, d);
            if !send_retry(storage, &req) {
                print_str("run: storage server did not answer\n");
                return;
            }
        }
        for i in 0..batch {
            let o = off + i * fs_proto::DATA_MAX;
            let want = core::cmp::min(fs_proto::DATA_MAX, size - o);
            let n = recv_data_chunk(inbox, want, (buf + o) as *mut u8);
            if n != want as u32 {
                print_str("run: read failed off=");
                print_u32(o as u32);
                print_char(b'\n');
                return;
            }
        }
        off += batch * fs_proto::DATA_MAX;
    }
    print_str("Running ");
    print_83(&raw);
    print_str("...\n");
    // On success this never returns: we wake up as the new program.
    let r = unsafe { exec(buf, size) };
    if r == userspace::EACCES {
        print_str("run: exec denied (no capability)\n");
    } else {
        print_str("run: exec failed (bad image)\n");
    }
}

fn cmd_signal(name: &[u8], arg: &[u8], sig: usize) {
    let say = |tail: &str| {
        for &b in name.iter() {
            print_char(b);
        }
        print_str(tail);
    };
    match cmd::atoi(arg) {
        Some(pid) => {
            let r = unsafe { kill(pid as usize, sig) };
            if r == 0 {
                say(" sent\n");
            } else {
                say(" failed\n");
            }
        }
        None => {
            say(": bad pid\n");
        }
    }
}

fn cmd_ps() {
    // Well-known servers by convention: 1 = VGA, 2 = keyboard, 3 = ISR,
    // 4 = storage. Plus our own PID.
    let mine = unsafe { syscall0(syscall_nr::GETPID) };
    let servers: [(&str, usize); 4] = [("vga", 1), ("kbd", 2), ("isr", 3), ("stor", 4)];
    for (label, which) in servers.iter() {
        let pid = unsafe { get_server_pid(*which) };
        print_str("  ");
        print_str(label);
        print_str(" pid=");
        if pid > 0 {
            print_u32(pid as u32);
        } else {
            print_str("-");
        }
        print_char(b'\n');
    }
    print_str("  self pid=");
    print_u32(mine as u32);
    print_char(b'\n');
}

fn dispatch(inbox: &mut Inbox, storage: usize, argv: &[[u8; cmd::MAX_ARG_LEN]; cmd::MAX_ARGS], lens: &[usize; cmd::MAX_ARGS], argc: usize) {
    let cmd_name = &argv[0][..lens[0]];
    let arg = |i: usize| -> &[u8] {
        if i < argc {
            &argv[i][..lens[i]]
        } else {
            &[]
        }
    };
    if cmd_name == b"ls" {
        cmd_ls(inbox, storage);
    } else if cmd_name == b"cat" {
        if argc < 2 {
            print_str("usage: cat FILE\n");
        } else {
            cmd_cat(inbox, storage, arg(1));
        }
    } else if cmd_name == b"echo" {
        for i in 1..argc {
            if i > 1 {
                print_char(b' ');
            }
            for &b in arg(i).iter() {
                print_char(b);
            }
        }
        print_char(b'\n');
    } else if cmd_name == b"clear" {
        unsafe { let _ = vga_clear(); }
    } else if cmd_name == b"run" {
        if argc < 2 {
            print_str("usage: run FILE\n");
        } else {
            cmd_run(inbox, storage, arg(1));
        }
    } else if cmd_name == b"kill" {
        if argc < 2 {
            print_str("usage: kill PID\n");
        } else {
            cmd_signal(b"kill", arg(1), sig::SIGTERM);
        }
    } else if cmd_name == b"stop" {
        if argc < 2 {
            print_str("usage: stop PID\n");
        } else {
            cmd_signal(b"stop", arg(1), sig::SIGSTOP);
        }
    } else if cmd_name == b"cont" {
        if argc < 2 {
            print_str("usage: cont PID\n");
        } else {
            cmd_signal(b"cont", arg(1), sig::SIGCONT);
        }
    } else if cmd_name == b"ps" {
        cmd_ps();
    } else if cmd_name == b"help" {
        print_str("commands: ls cat echo clear run kill stop cont ps help\n");
    } else {
        print_str("Unknown command: ");
        for &b in cmd_name.iter() {
            print_char(b);
        }
        print_char(b'\n');
    }
}

#[no_mangle]
pub extern "C" fn main() -> ! {
    // Discover the keyboard server PID via GET_SERVER_PID (server 2).
    let kbd_pid = unsafe { get_server_pid(2) };
    if kbd_pid <= 0 {
        // Keyboard server not available — halt.
        loop {}
    }
    let kbd_pid = kbd_pid as usize;
    // Storage server (server 4) backs ls/cat/run. It is spawned by the
    // kernel AFTER the shell, so a one-time lookup at startup would cache
    // "not present" forever: re-resolve on every command line instead.
    let mut storage = 0usize;

    // Line buffer for the current input line.
    let mut line_buf: [u8; LINE_BUF_SIZE] = [0; LINE_BUF_SIZE];
    let mut line_len: usize = 0;

    // Messages received while waiting for a different kind (see `recv_matching`).
    let mut inbox = Inbox::new();

    // Display initial prompt.
    print_str("Shell> ");

    loop {
        if let Some(ch) = get_key(&mut inbox, kbd_pid) {
            match ch {
                b'\n' | b'\r' => {
                    // Enter: run the line through the dispatcher.
                    print_char(b'\n');
                    let mut argv = [[0u8; cmd::MAX_ARG_LEN]; cmd::MAX_ARGS];
                    let mut lens = [0usize; cmd::MAX_ARGS];
                    let argc = cmd::parse_line(&line_buf[..line_len], &mut argv, &mut lens);
                    if argc > 0 {
                        let s = unsafe { get_server_pid(4) };
                        if s > 0 {
                            storage = s as usize;
                        }
                        dispatch(&mut inbox, storage, &argv, &lens, argc);
                    }
                    line_len = 0;
                    print_str("Shell> ");
                }
                0x08 => {
                    // Backspace: remove last character if any.
                    if line_len > 0 {
                        line_len -= 1;
                        print_char(0x08); // VGA cursor back
                    }
                }
                _ => {
                    // Regular character: append to line buffer and echo.
                    if line_len < LINE_BUF_SIZE - 1 {
                        line_buf[line_len] = ch;
                        line_len += 1;
                    }
                    print_char(ch);
                }
            }
        } else {
            // No key available — yield CPU cooperatively.
            unsafe { let _ = syscall0(syscall_nr::SCHEDULE); }
        }
    }
}
