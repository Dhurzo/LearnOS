# Phase 2: IPC Trace/Debug System

## Goal
Add observability to the IPC message passing system so students can see and debug cross-process communication in real-time. This transforms abstract concepts into observable phenomena.

## Context
- Current IPC uses ring buffers (IPC_QUEUE[10]) per process for message passing
- Messages are 64-byte IpcMessage structs with src_pid, msg_type, data fields
- No way to observe what messages are flowing between processes currently
- Kernel maintains timer ticks which can be used as timestamps

## Task Breakdown

### Task 2.1: Define IPC Trace Log Structure in ProcessTable
**Files:** kernel/src/process.rs
**Dependencies:** None (additive change)
**Tasks:**
- Add `ipc_trace: IpcTraceLog` field to ProcessTable struct
- Define IpcTraceLog struct with:
  - `entries: ArrayDeque<IpcTraceEntry, MAX_TRACE_SIZE>` where MAX_TRACE_SIZE=256
  - `max_size: u32` (configurable via syscall)
  - `current_size: u32` (tracks actual entries used)
- Define IpcTraceEntry struct: `{ timestamp: u32, src_pid: Pid, dst_pid: Pid, msg_type: u16, queue_depth_at_send: u8 }`

### Task 2.2: Modify IPC_SEND to Log Messages
**Files:** kernel/src/syscall.rs
**Dependencies:** Task 2.1
**Tasks:**
- Locate existing IPC_SEND syscall handler (syscall number 3)
- Add trace logging BEFORE the actual message delivery logic
- Capture timestamp from timer counter (use current tick count)
- Calculate queue_depth_at_send: read target process's IPC_QUEUE length before push
- Append new entry to ProcessTable.ipc_trace.entries ring buffer
- Ensure ring buffer wraps correctly when full (overwrite oldest entries)

### Task 2.3: Implement IPC_TRACE_QUERY Syscall
**Files:** kernel/src/syscall.rs, kernel/src/process.rs
**Dependencies:** Tasks 2.1, 2.2
**Tasks:**
- Add new syscall number `IPC_TRACE_QUERY = 9` to dispatch table
- Handler signature: `(count: u32) -> Vec<IpcTraceEntry>`
- Return up to `count` most recent entries from calling process's trace log
- If count > current_size, return all available entries
- Entries should be returned in chronological order (oldest first in the batch)

### Task 2.4: Implement IPC_QUEUE_DEPTH Syscall
**Files:** kernel/src/syscall.rs
**Dependencies:** None new (uses existing ProcessTable structure)
**Tasks:**
- Add new syscall number `IPC_QUEUE_DEPTH = 10` to dispatch table
- Handler signature: `(target_pid: Pid) -> u8`
- Validate target_pid is within valid range and not self
- Read current IPC_QUEUE length for target process (use atomic read or lock)
- Return queue occupancy as percentage (0-255) or absolute count (choose one approach)

### Task 2.5: Implement IPC_TRACE_CONFIG Syscall
**Files:** kernel/src/syscall.rs, kernel/src/process.rs
**Dependencies:** Tasks 2.1, 2.3
**Tasks:**
- Add new syscall number `IPC_TRACE_CONFIG = 11` to dispatch table
- Handler signature: `(max_size: u32) -> Result<(), ConfigError>`
- Validate max_size is within reasonable bounds (e.g., 64 to 4096 entries)
- Update ProcessTable.ipc_trace.max_size
- If new size < current_size, truncate the trace log to fit new limit
- Return error if invalid size provided

### Task 2.6: Ensure Trace Logging Doesn't Affect Normal Operation
**Files:** kernel/src/syscall.rs
**Dependencies:** All previous tasks
**Tasks:**
- Verify trace logging adds minimal overhead (single array push operation)
- Confirm ring buffer operations don't interfere with IPC_QUEUE logic
- Test that normal message passing still works correctly with tracing enabled
- Add optional enable/disable flag to IPC_TRACE_CONFIG if needed for performance

## Acceptance Criteria

### Test 2.6.1: Trace Captures Basic Message Flow
**Steps:**
1. Process A sends message to Process B via IPC_SEND (msg_type=1)
2. Process C queries trace log with count=10
3. Verify trace contains entry with src_pid=A, dst_pid=B, msg_type=1

**Expected:** At least one trace entry captured for the IPC_SEND operation

### Test 2.6.2: Trace Shows Queue Depth at Send Time
**Steps:**
1. Process B has 5 messages already in queue
2. Process A sends message to Process B (msg_type=2)
3. Process C queries trace log and reads queue_depth_at_send field for that entry

**Expected:** queue_depth_at_send should be 5 (not 6, as it captures depth BEFORE push)

### Test 2.6.3: Trace Log Wraps Correctly When Full
**Steps:**
1. Configure trace max_size=5 via IPC_TRACE_CONFIG
2. Send 10 different messages between various processes
3. Query trace log with count=10
4. Verify only 5 most recent entries are returned (oldest ones discarded)

**Expected:** Ring buffer overwrites oldest entries, returns exactly max_size entries

### Test 2.6.4: IPC_QUEUE_DEPTH Returns Accurate Counts
**Steps:**
1. Process A sends 3 messages to Process B via IPC_SEND
2. Process C calls IPC_QUEUE_DEPTH(Process B's PID)
3. Verify returned count matches actual queue length (should be 3)

**Expected:** Queue depth reflects current state at time of query, not stale data

### Test 2.6.5: Trace Doesn't Break Normal IPC
**Steps:**
1. Enable tracing via IPC_TRACE_CONFIG with max_size=100
2. Run normal IPC_SEND/IPC_RECV operations between multiple processes
3. Verify all messages are delivered correctly and no deadlocks occur
4. Compare performance: trace-enabled vs disabled (should be similar overhead)

**Expected:** Normal message passing continues to work without errors or significant slowdown

### Test 2.6.6: Multiple Processes Can Query Their Own Trace Logs Independently
**Steps:**
1. Process A sends to B, C sends to D
2. Process A queries trace → sees only messages it sent/received
3. Process C queries trace → sees only messages it sent/received

**Expected:** Each process maintains its own trace log (not global), isolated per-process

## Files to Modify/Create

### Modified:
- `kernel/src/process.rs`: Add IpcTraceLog structure and fields to ProcessTable
- `kernel/src/syscall.rs`: 
  - Modify IPC_SEND handler to log messages
  - Add IPC_TRACE_QUERY syscall handler (number 9)
  - Add IPC_QUEUE_DEPTH syscall handler (number 10)
  - Add IPC_TRACE_CONFIG syscall handler (number 11)

### Created:
- None required (all functionality fits in existing files)

## Integration Notes
- Trace logging is purely additive - no changes to IPC message delivery logic
- Ring buffer uses ArrayDeque pattern (existing in Rust stdlib or implement simple version)
- Trace log is per-process, not global - each process tracks its own communications
- Timestamps use timer tick counter (already available from IRQ0 handler)
- No impact on paging, memory allocation, or scheduler behavior
