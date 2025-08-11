# Inter-Process Communication

<!--
Put system calls such as
msgget, msgsnd, msgrcv, msgctl, semget, semop, semctl, shmget, shmat, shmctl
futex, set_robust_list, and get_robust_list
under this category.
-->

## `System V semaphore`

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
```

Unsupported commands:
* `IPC_STAT`
* `IPC_INFO`
* `SEM_INFO`
* `SEM_STAT`
* `SEM_STAT_ANY`
* `GETALL`
* `SETALL`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/semctl.2.html).