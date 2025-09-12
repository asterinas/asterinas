# Inter-Process Communication

<!--
Put system calls such as
msgget, msgsnd, msgrcv, msgctl, semget, semop, semctl, shmget, shmat, shmctl
futex, set_robust_list, and get_robust_list
under this category.
-->

## System V semaphore

### `semget`

Supported functionality in SCML:

```c
// Creat or open a semaphore set
semget(
    key,
    nsems,
    semflg = IPC_CREAT | IPC_EXCL
);
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/semget.2.html).

### `semop` and `semtimedop`

Supported functionality in SCML:

```c
struct sembuf = {
    sem_flg = IPC_NOWAIT,
    ..
};

// Semaphore operations without blocking
semop(
    semid,
    sops = [ <sembuf> ],
    nsops
);
```

Unsupported semaphore flags:
* `SEM_UNDO`

Supported and unsupported functionality of `semtimedop` are the same as `semop`.
The SCML rules are omitted for brevity.

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/semop.2.html).

### `semctl`

Supported functionality in SCML:

```c
// Remove the semaphore set
semctl(
    semid,
    semnum,
    cmd = IPC_RMID
);

// Initialize the value of the semnum-th semaphore
semctl(
    semid,
    semnum,
    cmd = SETVAL,
    arg
);

// Return the current value (GETVAL), last operating process's PID (GETPID),
// count of processes awaiting increment (GETNCNT) or count of processes awaiting
// zero (GETZCNT) of the semnum-th semaphore
semctl(
    semid,
    semnum,
    cmd = GETVAL | GETPID | GETNCNT | GETZCNT
);

// Retrieve a copy of the `semid_ds` kernel structure for the specified semaphore set
semctl(
    semid,
    semnum,
    cmd = IPC_STAT,
    arg
);
```

Unsupported commands:
* `IPC_INFO`
* `SEM_INFO`
* `SEM_STAT`
* `SEM_STAT_ANY`
* `GETALL`
* `SETALL`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/semctl.2.html).

### `futex`

Supported functionality in SCML:

```c
opt_flags = FUTEX_PRIVATE_FLAG | FUTEX_CLOCK_REALTIME;

// Block current thread if target value at `uaddr` matches `val`, and wait up to `timeout`.
futex(
    uaddr,
    futex_op = FUTEX_WAIT | <opt_flags>,
    val, timeout
);

// Block current thread with bitmask condition if target value at `uaddr` matches `val`,
// and wait up to `timeout`.
futex(
    uaddr,
    futex_op = FUTEX_WAIT_BITSET | <opt_flags>,
    val, timeout, unused = NULL, bitmask
);

// Unblock up to `max_waiters` threads waiting on `uaddr`
futex(
    uaddr,
    futex_op = FUTEX_WAKE | <opt_flags>,
    max_waiters
);

// Unblock up to `max_waiters` threads on `uaddr`, if the value on `uaddr` matches `bitmask`
futex(
    uaddr,
    futex_op = FUTEX_WAKE_BITSET | <opt_flags>,
    max_waiters, unused0 = NULL, unused1 = NULL, bitmask
);

// Unblock up to `max_waiters` threads waiting on `uaddr`, and requeue up to
// `max_requeue_waiters` of the remaining waiters to the target futex at `uaddr2`.
futex(
    uaddr,
    futex_op = FUTEX_REQUEUE | <opt_flags>,
    max_waiters, max_requeue_waiters, uaddr2
);

// Perform atomic operation encoded in `operation` on `uaddr2`. Unblock up to `max_waiters`
// threads waiting on `uaddr`, and conditionally unblock up to `max_waiters2` threads
// waiting on `uaddr2` based on the result of the atomic operation.
futex(
    uaddr,
    futex_op = FUTEX_WAKE_OP | <opt_flags>,
    max_waiters, max_waiters2, uaddr2, operation
);
```

Unsupported operations:
* `FUTEX_FD`
* `FUTEX_CMP_REQUEUE`
* `FUTEX_LOCK_PI`
* `FUTEX_UNLOCK_PI`
* `FUTEX_TRYLOCK_PI`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/futex.2.html).