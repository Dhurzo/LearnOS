# Phase 1: IPC Synchronization Primitives

## Goal
Add semaphore/mutex synchronization primitives to enable cross-process coordination in userspace servers. This is foundational - all subsequent features depend on being able to synchronize between processes.

## Context
- Current IPC system uses ring buffers (IPC_QUEUE[10]) for message passing
- ProcessTable has state transitions: Running → Ready → Blocked
- Scheduler handles context switching via timer interrupt
- No existing synchronization primitives exist in the codebase

## Task Breakdown

### Task 1.1: Extend ProcessTable with Semaphore Data Structures
**Files:** kernel/src/process.rs
**Dependencies:** None (additive change)
**Tasks:**
- Add `semaphores: HashMap<SemaphoreId, Semaphore>` to ProcessTable struct
- Define Semaphore struct: `{ name: String, count: u32, wait_queue: Vec<Pid> }`
- Initialize semaphore table as empty HashMap on process creation

### Task 1.2: Implement IPC_SEMAPHORE_INIT Syscall Handler
**Files:** kernel/src/syscall.rs
**Dependencies:** Task 1.1
**Tasks:**
- Add new syscall number `IPC_SEMAPHORE_INIT = 5` to dispatch table
- Handler signature: `(name: &str, initial_count: u32) -> SemaphoreId`
- Create new semaphore with given name and initial count
- Store in calling process's semaphore table
- Return unique ID (use atomic counter or hash of name+pid)

### Task 1.3: Implement IPC_WAIT Syscall Handler
**Files:** kernel/src/syscall.rs, kernel/src/process.rs
**Dependencies:** Tasks 1.1, 1.2
**Tasks:**
- Add new syscall number `IPC_WAIT = 6` to dispatch table
- Handler signature: `(semaphore_id: SemaphoreId) -> Result<(), WaitError>`
- Decrement semaphore count; if count < 0 after decrement:
  - Add calling PID to semaphore's wait queue
  - Transition process state to Blocked
  - Trigger scheduler to pick next ready process
- If count >= 0: return Ok(()) immediately

### Task 1.4: Implement IPC_SIGNAL Syscall Handler
**Files:** kernel/src/syscall.rs, kernel/src/process.rs
**Dependencies:** Tasks 1.2, 1.3
**Tasks:**
- Add new syscall number `IPC_SIGNAL = 7` to dispatch table
- Handler signature: `(semaphore_id: SemaphoreId) -> Result<(), SignalError>`
- Increment semaphore count; if wait queue is non-empty:
  - Pop first PID from wait queue
  - Transition that process state from Blocked → Ready
  - Add to scheduler's ready queue
  - Set flag to trigger context switch on next timer tick
- If no waiters: just increment count

### Task 1.5: Handle Process Termination with Semaphore Cleanup
**Files:** kernel/src/process.rs, kernel/src/syscall.rs
**Dependencies:** All previous tasks
**Tasks:**
- When process is killed/terminated:
  - Remove all semaphores owned by that PID from ProcessTable
  - Wake all waiters on those semaphores (transition Blocked → Ready)
  - Prevent dangling semaphore references

### Task 1.6: Add Semaphore Query Syscall
**Files:** kernel/src/syscall.rs
**Dependencies:** All previous tasks
**Tasks:**
- Add new syscall number `IPC_SEMAPHORE_QUERY = 8` to dispatch table
- Handler signature: `(semaphore_id: SemaphoreId) -> (count, waiters)` tuple
- Returns current semaphore state for debugging/monitoring

## Acceptance Criteria

### Test 1.6.1: Basic Semaphore Operations
**Steps:**
1. Process A creates semaphore "test_sem" with count=0 via IPC_SEMAPHORE_INIT
2. Process B calls IPC_WAIT on "test_sem" → should block (count decremented to -1)
3. Process A calls IPC_SIGNAL on "test_sem" → Process B should transition to Ready
4. Process B resumes execution and continues

**Expected:** Process B blocks when count would go negative, unblocks immediately after signal

### Test 1.6.2: Multiple Waiters
**Steps:**
1. Create semaphore with count=0
2. Processes A, B, C all call IPC_WAIT → all three should block (count=-3)
3. Process X calls IPC_SIGNAL once → exactly one waiter (A) should unblock
4. Call IPC_SIGNAL twice more → B and C should unblock in order

**Expected:** FIFO ordering of waiters, each signal wakes exactly one process

### Test 1.6.3: Semaphore with Initial Count > 0
**Steps:**
1. Create semaphore "resource" with count=2
2. Process A calls IPC_WAIT → count becomes 1, returns Ok (no blocking)
3. Process B calls IPC_WAIT → count becomes 0, returns Ok (no blocking)
4. Process C calls IPC_WAIT → count becomes -1, should block

**Expected:** Processes don't block until count goes negative

### Test 1.6.4: Signal Without Waiters
**Steps:**
1. Create semaphore with count=5
2. Call IPC_SIGNAL multiple times (total signals > initial count)
3. Verify semaphore count increases correctly without errors

**Expected:** Count can go above initial value, no errors when signaling empty wait queue

### Test 1.6.5: Process Termination Cleanup
**Steps:**
1. Process A owns semaphore "shared" with count=0
2. Process B waits on "shared" → blocks (count=-1)
3. Kill Process A
4. Verify Process B unblocks and can continue execution

**Expected:** Deadlock broken when owner dies, waiters transition to Ready state

## Files to Modify/Create

### Modified:
- `kernel/src/process.rs`: Add semaphore data structures to ProcessTable, implement wait/signal logic
- `kernel/src/syscall.rs`: Add 4 new syscall handlers (IPC_SEMAPHORE_INIT=5, IPC_WAIT=6, IPC_SIGNAL=7, IPC_SEMAPHORE_QUERY=8)

### Created:
- None required (all functionality fits in existing files)

## Integration Notes
- Semaphore wait/signal operations integrate with existing Blocked/Ready state transitions
- Scheduler continues to pick from ready queue as before; new Ready processes are added normally
- No changes needed to paging, memory allocation, or IRQ handling
- Existing IPC message passing remains unchanged
