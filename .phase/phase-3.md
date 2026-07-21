# Phase 3: Userspace Shell/REPL

## Goal
Create an interactive userspace shell program that allows students to experiment with and observe the microkernel system. This provides hands-on interaction with processes, IPC, semaphores, and trace logs.

## Context
- Userspace programs are loaded as ELF binaries (ELF loader already implemented in main.rs)
- Existing userspace infrastructure: vga_server.rs, keyboard_server.rs in userspace/src/
- VGA server handles display output via IPC to kernel
- Keyboard server receives scancodes and routes to focused process
- New capabilities available from Phases 1 & 2: semaphore ops, trace query, queue depth

## Task Breakdown

### Task 3.1: Create Shell Program Skeleton
**Files:** userspace/src/shell.rs (NEW)
**Dependencies:** None (additive, follows existing userspace pattern)
**Tasks:**
- Create shell.rs with main() entry point similar to vga_server.rs structure
- Initialize shell state struct: `{ pid: u16, name: String, focused_window: Option<Pid> }`
- Set up basic message loop pattern (recv messages from keyboard server → parse commands → execute)

### Task 3.2: Implement Command Parsing Framework
**Files:** userspace/src/shell.rs
**Dependencies:** Task 3.1
**Tasks:**
- Define command structure: `enum ShellCommand { Ps, IpcTrace(u32), Kill(Pid), Msg(Pid, u16, [u8;56]), SemInit(String, u32), SemWait(u32), SemSignal(u32) }`
- Implement parse_command(line: &str) -> Result<ShellCommand, ParseError> function
- Handle command syntax with basic tokenization (split by whitespace)
- Support command aliases and help text generation

### Task 3.3: Implement 'ps' Command
**Files:** userspace/src/shell.rs
**Dependencies:** Task 3.2
**Tasks:**
- Add syscall IPC_PROCESS_LIST or extend existing query mechanism to retrieve process table state
- For each process, display: PID, name (if available), current state (Running/Ready/Blocked)
- Format output as table with headers aligned for readability on VGA text mode
- Handle edge case where shell itself is in list (mark with * or similar indicator)

**Note:** If no existing syscall provides process listing, add IPC_PROCESS_LIST = 12 syscall to kernel that returns serialized ProcessTable state. Implementation should return array of {pid, name, state} structs.

### Task 3.4: Implement 'ipc trace N' Command
**Files:** userspace/src/shell.rs
**Dependencies:** Tasks 3.2, Phase 2 completion
**Tasks:**
- Call IPC_TRACE_QUERY syscall with parameter N from command line
- Format returned trace entries into readable display format:
  ```
  [timestamp] PID_A → PID_B (msg_type=1) queue_depth=5
  ```
- Display up to N most recent messages, oldest first in the batch

### Task 3.5: Implement 'kill PID' Command
**Files:** userspace/src/shell.rs
**Dependencies:** Tasks 3.2, Phase 1 completion
**Tasks:**
- Call existing kill syscall (or add if not exists) with target PID
- Display confirmation message "Killed process N" or error if invalid PID
- Handle permission check (only root/special processes can kill others - define policy)

### Task 3.6: Implement 'msg target type data' Command
**Files:** userspace/src/shell.rs
**Dependencies:** Tasks 3.2, Phase 1 completion
**Tasks:**
- Parse three arguments: target_pid (u16), msg_type (u16), data (string converted to [u8;56])
- Call IPC_SEND syscall with parsed parameters
- Display confirmation "Sent message type N to PID M" or error if send failed

### Task 3.7: Implement 'sem create/wait/signal' Commands
**Files:** userspace/src/shell.rs
**Dependencies:** Tasks 3.2, Phase 1 completion
**Tasks:**
- **sem create NAME COUNT**: Call IPC_SEMAPHORE_INIT syscall with name and initial count
  - Display "Created semaphore X (ID=N)" where N is returned ID
  - Store semaphore IDs in shell's local registry for subsequent use
  
- **sem wait ID**: Call IPC_WAIT syscall with semaphore ID
  - Block until semaphore signaled (shell enters wait state)
  - Display "Acquired semaphore N" when successful
  
- **sem signal ID**: Call IPC_SIGNAL syscall with semaphore ID
  - Display "Signaled semaphore N" after call returns

### Task 3.8: Handle Input/Output Integration
**Files:** userspace/src/shell.rs, kernel/src/syscall.rs (if needed)
**Dependencies:** All previous tasks
**Tasks:**
- Integrate with existing keyboard server infrastructure:
  - Shell registers as focused process via IPC message to keyboard server
  - Keyboard server routes scancodes to shell when focused
  
- Display prompt on VGA screen before each command input:
  ```
  learnos> 
  ```
  
- Handle special keys:
  - Enter: execute parsed command
  - Backspace: delete last character from input buffer
  - Ctrl+C: cancel current command (clear input buffer)

### Task 3.9: Add Shell to Boot Sequence
**Files:** kernel/src/main.rs
**Dependencies:** All previous tasks
**Tasks:**
- Modify main.rs ELF loader logic to also load shell.elf at boot
- Load shell at appropriate memory location (similar to vga_server loading pattern)
- Initialize shell process with PID (assign after VGA and keyboard servers, e.g., PID=3)
- Ensure shell is set as "focused" by default so it receives keyboard input

### Task 3.10: Implement 'help' Command
**Files:** userspace/src/shell.rs
**Dependencies:** All previous tasks
**Tasks:**
- Display list of available commands with brief descriptions:
  ```
  Available commands:
    ps                    - List all processes and their states
    ipc trace N           - Show last N IPC messages in trace log
    kill PID              - Terminate process with given ID
    msg TARGET TYPE DATA  - Send IPC message to another process
    sem create NAME COUNT - Create new semaphore (initial count=COUNT)
    sem wait ID           - Wait on semaphore (blocks if count<=0)
    sem signal ID         - Signal semaphore (wakes one waiter)
    help                  - Display this help text
  ```

## Acceptance Criteria

### Test 3.10.1: Shell Loads and Displays Prompt
**Steps:**
1. Boot system with all phases implemented
2. Verify VGA screen shows "learnos> " prompt at startup
3. Type any character → should appear after prompt on screen

**Expected:** Shell loads as userspace process, receives keyboard input, displays typed characters immediately

### Test 3.10.2: 'ps' Command Lists Processes Correctly
**Steps:**
1. Boot system (VGA server PID=1, keyboard server PID=2 should exist)
2. Type "ps" and press Enter
3. Verify output shows at least two processes with PIDs and states

**Expected:** Output displays process table state including shell itself, with accurate PID numbers and state labels

### Test 3.10.3: 'ipc trace N' Shows Recent Messages
**Steps:**
1. Create another userspace program that sends IPC messages to VGA server
2. After sending several messages, type "ipc trace 5" in shell
3. Verify output shows 5 most recent trace entries with correct src/dst PIDs and msg types

**Expected:** Trace log populated by other processes' IPC operations, displayed in chronological order

### Test 3.10.4: 'msg' Command Sends Messages Successfully
**Steps:**
1. VGA server receives messages via existing IPC infrastructure
2. In shell, type "msg 1 99 test_data" (send msg_type=99 with data to PID=1)
3. Verify VGA server processes the message correctly (displays "test_data" or similar response)

**Expected:** Message delivered to target process via existing IPC_SEND mechanism

### Test 3.10.5: Semaphore Operations Work End-to-End
**Steps:**
1. Create second userspace program that waits on semaphore ID=0
2. In shell, type "sem create shared 0" → creates semaphore with count=0
3. Second program calls IPC_WAIT on sem ID=0 → should block
4. In shell, type "sem signal 0" → second program should unblock and continue

**Expected:** Shell can create semaphores and trigger signals that affect other processes' execution state

### Test 3.10.6: 'kill PID' Terminates Target Process
**Steps:**
1. Create test userspace program running in loop (PID=N)
2. Type "ps" → verify N appears in process list
3. Type "kill N" → should terminate that process
4. Type "ps" again → N no longer appears in list

**Expected:** Target process removed from ProcessTable, resources freed, subsequent ps shows updated list

### Test 3.10.7: 'help' Command Displays Documentation
**Steps:**
1. Boot system and type "help" at prompt
2. Verify output lists all available commands with descriptions

**Expected:** Help text formatted clearly, covers all implemented commands, easy to read on VGA text mode

### Test 3.10.8: Shell Handles Invalid Commands Gracefully
**Steps:**
1. Type various malformed commands: "ps extra_arg", "kill abc", "msg 1 invalid", etc.
2. Verify shell displays error messages instead of crashing or doing nothing

**Expected:** Error handling for each command type, informative error messages displayed to user

## Files to Modify/Create

### Created:
- `userspace/src/shell.rs` - Main shell program with all command implementations

### Modified:
- `kernel/src/main.rs` - Add shell.elf loading to boot sequence (assign PID=3)
- `kernel/src/syscall.rs` - Add IPC_PROCESS_LIST syscall (number 12) if not already present for process table queries
- Any existing syscall dispatch code that needs integration with new commands

## Integration Notes
- Shell follows same ELF loading pattern as vga_server and keyboard_server
- Uses existing userspace infrastructure (keyboard server routing, VGA display via IPC)
- Command execution calls existing or newly-added syscalls from Phases 1 & 2
- No kernel changes required beyond optional process listing syscall
- Shell runs as regular userspace process with full access to all microkernel services
