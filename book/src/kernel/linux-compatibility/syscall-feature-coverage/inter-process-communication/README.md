# Inter-Process Communication

<!--
Put system calls such as
msgget, msgsnd, msgrcv, msgctl, semget, semop, semctl, shmget, shmat, shmctl
futex, set_robust_list, and get_robust_list
under this category.
-->

### `futex`

Supported functionality in SCML:

```c
{{#include futex.scml}}
```

Unsupported operations:
* `FUTEX_FD`
* `FUTEX_CMP_REQUEUE`
* `FUTEX_LOCK_PI`
* `FUTEX_UNLOCK_PI`
* `FUTEX_TRYLOCK_PI`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/futex.2.html).

## System V semaphore

### `semget`

Supported functionality in SCML:

```c
{{#include semget.scml}}
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/semget.2.html).

### `semop` and `semtimedop`

Supported functionality in SCML:

```c
{{#include semop_and_semtimedop.scml}}
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
{{#include semctl.scml}}
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
