# Phase 4: Process Signals (Asynchronous Notifications)

## Goal
Add asynchronous signal notification system enabling processes to send non-blocking alerts to other processes without waiting for acknowledgment. This teaches real-world async inter-process communication patterns like SIGTERM, SIGUSR1, etc.

## Context
- Existing IPC uses synchronous message passing (IPC_SEND blocks until delivered, IPC_RECV waits for next message)
- No mechanism for one-way notifications where sender doesn't wait for receiver to act
- Kill syscall conceptually exists but isn't formalized as a real signal delivery
- Userspace programs currently use polling or blocking recv patterns exclusively

## Task Breakdown

### Task 4.1: Define Signal Data Structures in ProcessTable
**Files:** kernel/src/process.rs
**Dependencies:** None (additive change)
**Tasks:**
- Add `signal_mask: u32` field to process state struct (bitmask of blocked signals)
- Add `pending_signals: u32` field tracking signals currently queued for delivery
- Define MAX_SIGNALS constant = 16 (enough for common Unix-style signals)
- Create Signal enum or constants mapping signal numbers to meanings:
  - SIGKILL = 9, SIGTERM = 15, SIGUSR1 = 10, SIGUSR2 = 12, etc.

### Task 4.2: Implement IPC_SIGNAL_PROCESS Syscall (Signal Delivery)
**Files:** kernel/src/syscall.rs
**Dependencies:** Task 4.1
**Tasks:**
- Add new syscall number `IPC_SIGNAL_PROCESS = 13` to dispatch table
- Handler signature: `(target_pid: Pid, signal_number: u8) -> Result<(), SignalError>`
- Validate target_pid exists and is not self (signals can't be sent to self synchronously via this syscall - use different mechanism if needed)
- Set bit in target.process.pending_signals for given signal number
- Trigger async notification mechanism (don't block sender, just queue the signal)

### Task 4.3: Implement Signal Delivery Mechanism on Context Switch/IRQ
**Files:** kernel/src/syscall.rs, kernel/src/process.rs
**Dependencies:** Tasks 4.1, 4.2
**Tasks:**
- When process transitions from Blocked → Ready (via scheduler pick or signal unblock):
  - Check if pending_signals has any bits set
  - If yes: transition to a new "SignalPending" state or handle immediately in Ready state
  - Deliver signals by invoking userspace signal handler registration mechanism
  
**Alternative Approach (Simpler):**
- When process resumes execution after context switch:
  - Kernel checks pending_signals bitmask before returning to user mode via sysretq
  - If any signals pending, push signal info onto userspace stack and set RIP to signal handler address (if registered) or default handler
  
**Recommended Approach for Didactic Clarity:**
- Add new process state: `WaitingForSignal` (similar to Blocked but triggered by signals)
- When process calls IPC_WAIT_SIGNAL syscall, it blocks until a matching signal arrives
- Signal delivery sets bit in pending_signals and wakes the waiting process

### Task 4.4: Implement IPC_REGISTER_SIGNAL_HANDLER Syscall
**Files:** kernel/src/syscall.rs, userspace/src/shell.rs (example usage)
**Dependencies:** Tasks 4.1, 4.2
**Tasks:**
- Add new syscall number `IPC_REGISTER_SIGNAL_HANDLER = 14` to dispatch table
- Handler signature: `(signal_number: u8, handler_address: usize) -> Result<(), RegError>`
- Store handler function pointer in process's signal handler registry (array of usize indexed by signal number)
- Allow userspace programs to register custom handlers for specific signals
- Default handler for SIGKILL/SIGTERM = terminate process immediately

### Task 4.5: Implement IPC_BLOCK_SIGNAL and IPC_UNBLOCK_SIGNAL Syscalls
**Files:** kernel/src/syscall.rs
**Dependencies:** Task 4.1
**Tasks:**
- Add new syscall number `IPC_BLOCK_SIGNAL = 15` to dispatch table
- Handler signature: `(signal_number: u8) -> Result<(), BlockError>`
- Set bit in process.signal_mask for given signal number (mask=blocked)

- Add new syscall number `IPC_UNBLOCK_SIGNAL = 16` to dispatch table  
- Handler signature: `(signal_number: u8) -> Result<(), UnblockError>`
- Clear bit in process.signal_mask for given signal number (unblock/deliverable)

### Task 4.6: Formalize SIGKILL as Real Signal
**Files:** kernel/src/syscall.rs, kernel/src/process.rs
**Dependencies:** All previous tasks
**Tasks:**
- Modify existing kill syscall implementation to send SIGKILL via IPC_SIGNAL_PROCESS instead of direct termination
- Verify SIGKILL delivery triggers immediate process termination regardless of signal_mask
- Add test case showing SIGKILL cannot be blocked (mask ignored for this specific signal)

### Task 4.7: Implement Signal Delivery on IRQ/Timer Interrupt
**Files:** kernel/src/syscall.rs, kernel/src/process.rs
**Dependencies:** Tasks 4.1, 4.2, 4.3
**Tasks:**
- When timer interrupt (IRQ0) triggers context switch:
  - After picking next ready process from scheduler queue, check its pending_signals
  - If signals pending and handler registered: inject signal delivery before returning to user mode
  - Implementation: temporarily push signal info onto kernel stack, jump to userspace handler address via sysretq
  
**Note:** This is complex. For didactic simplicity, consider implementing "signal polling" instead where process must explicitly check for pending signals via IPC_POLL_SIGNALS syscall (number 17). This avoids the complexity of interrupt-driven signal injection and teaches explicit vs implicit async patterns.

### Task 4.8: Extend Shell with Signal Commands
**Files:** userspace/src/shell.rs
**Dependencies:** All previous tasks, Phase 3 completion
**Tasks:**
- Add 'signal send TARGET SIGNAL_NUM' command to shell
  - Parse target_pid and signal_number from arguments
  - Call IPC_SIGNAL_PROCESS syscall with parsed values
  
- Add 'signal block SIGNAL_NUM' command to shell
  - Block incoming signals of given type for shell process itself
  
- Add 'signal unblock SIGNAL_NUM' command to shell
  - Unblock previously blocked signals

- Add 'signal status' command to shell (optional, advanced)
  - Display current signal mask and pending signals for shell process
  - Format: "Mask: [SIGUSR1 blocked, SIGTERM delivered], Pending: [SIGUSR2]"

### Task 4.9: Create Example Userspace Signal Handler Program
**Files:** userspace/src/signal_demo.rs (NEW)
**Dependencies:** All previous tasks
**Tasks:**
- Create simple demonstration program showing signal handling in action
- Register handler for SIGUSR1 that prints "Received SIGUSR1!" to VGA display via IPC
- Main loop: sleep/wait until signal received, then print message and continue
- Load at boot alongside other userspace programs (PID assigned after shell)
- Demonstrates real-world async notification pattern

## Acceptance Criteria

### Test 4.9.1: Basic Signal Delivery Works
**Steps:**
1. Create two userspace processes (A and B)
2. Process A calls IPC_SIGNAL_PROCESS(Process B's PID, SIGUSR1)
3. Verify Process B receives the signal notification (via pending_signals bit set or handler invoked)

**Expected:** Signal queued for delivery to target process without sender blocking

### Test 4.9.2: Signal Masking Prevents Delivery
**Steps:**
1. Process A registers handler for SIGUSR1
2. Process A calls IPC_BLOCK_SIGNAL(SIGUSR1) → mask bit set
3. Process B sends SIGUSR1 to Process A via IPC_SIGNAL_PROCESS
4. Verify signal does NOT trigger handler invocation (mask prevents delivery)
5. Process A calls IPC_UNBLOCK_SIGNAL(SIGUSR1) → mask bit cleared
6. Signal should now be delivered immediately or queued for next context switch

**Expected:** Masked signals are held in pending_signals until unblocked, then delivered promptly

### Test 4.9.3: SIGKILL Cannot Be Blocked
**Steps:**
1. Process A calls IPC_BLOCK_SIGNAL(SIGKILL) → attempt to mask kill signal
2. Process B sends SIGKILL to Process A via IPC_SIGNAL_PROCESS
3. Verify Process A terminates immediately despite block attempt

**Expected:** SIGKILL bypasses signal_mask, process termination is mandatory and immediate

### Test 4.9.4: Multiple Signals Can Be Pending Simultaneously
**Steps:**
1. Process A registers handlers for both SIGUSR1 and SIGUSR2
2. Process B sends SIGUSR1 to A, then SIGUSR2 to A in rapid succession
3. Verify both signals are queued in pending_signals bitmask (bits 10 and 12 set)
4. When delivered, both handlers should execute (order may vary but both must fire)

**Expected:** Bitmask allows multiple concurrent pending signals, all delivered when unblocked

### Test 4.9.5: Signal Handler Executes in Userspace Context
**Steps:**
1. Create signal_demo program that registers SIGUSR1 handler printing to VGA
2. Send SIGUSR1 from another process via IPC_SIGNAL_PROCESS
3. Verify VGA displays "Received SIGUSR1!" message

**Expected:** Custom userspace signal handler invoked successfully, executes in Ring 3 context with access to its own memory space

### Test 4.9.6: Shell Can Monitor and Control Signals
**Steps:**
1. Boot system with all phases implemented
2. Type "signal block SIGUSR1" in shell → verify mask updated (display status if available)
3. Type "signal send N SIGUSR1" targeting some process → verify signal delivered to target
4. Type "signal unblock SIGUSR1" → verify blocking removed

**Expected:** Shell provides full control over signal operations, can both send and manage its own signal state

### Test 4.9.7: Signal Demo Program Works End-to-End
**Steps:**
1. Boot system with signal_demo loaded as userspace process (PID=M)
2. Type "signal send M SIGUSR1" in shell
3. Verify VGA displays "Received SIGUSR1!" output from signal_demo program

**Expected:** Complete async notification cycle works: sender → kernel queue → receiver handler execution → visible output

### Test 4.9.8: Signal Polling Alternative Works (if implemented instead of interrupt-driven)
**Steps:**
1. Process A registers handler for SIGUSR1
2. Process B sends SIGUSR1 to A
3. Process A calls IPC_POLL_SIGNALS → returns bitmask with SIGUSR1 bit set
4. Process A invokes its registered handler explicitly based on poll result

**Expected:** Explicit polling mechanism works as alternative to automatic signal injection, teaches different async pattern

## Files to Modify/Create

### Created:
- `userspace/src/signal_demo.rs` - Example userspace program demonstrating signal handling

### Modified:
- `kernel/src/process.rs`: Add signal_mask, pending_signals fields and related data structures
- `kernel/src/syscall.rs`: 
  - Add IPC_SIGNAL_PROCESS syscall (number 13)
  - Add IPC_REGISTER_SIGNAL_HANDLER syscall (number 14)
  - Add IPC_BLOCK_SIGNAL syscall (number 15)
  - Add IPC_UNBLOCK_SIGNAL syscall (number 16)
  - Optionally add IPC_POLL_SIGNALS syscall (number 17) for polling alternative
  - Modify existing kill implementation to use SIGKILL signal delivery

- `userspace/src/shell.rs`: Add signal-related commands (signal send/block/unblock/status)

## Integration Notes
- Signal system is additive to existing synchronous IPC - no changes to message passing logic
- pending_signals bitmask integrates with existing process state tracking
- Signal handlers execute in userspace context, respecting ring separation
- SIGKILL formalization may require minor adjustments to kill syscall but maintains same user-facing behavior
- Didactic note: Consider implementing signal polling (IPC_POLL_SIGNALS) as primary mechanism rather than interrupt-driven injection - simpler to understand and debug for students
