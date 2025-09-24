# Process & Thread Management

<!--
Put system calls such as
fork, vfork, clone, execve, exit, exit_group, wait4, waitid,
getpid, getppid, gettid, setuid, setgid, getuid, getgid, and prctl
under this category.
-->

## `sched_getattr` and `sched_setattr`

Supported functionality in SCML:

```c
// Get the scheduling policy of a "normal" thread
sched_getattr(
    pid,
    attr = {
        sched_policy = SCHED_OTHER | SCHED_BATCH | SCHED_IDLE,
        sched_flags = 0,
        ..
    },
    flags = 0,
);
// Set the scheduling policy of a "normal" thread
sched_setattr(
    pid,
    attr = {
        sched_policy = SCHED_OTHER | SCHED_BATCH | SCHED_IDLE,
        sched_flags = 0,
        ..
    },
    flags = 0,
);

// Get the scheduling policy of a real-time thread
sched_getattr(
    pid,
    attr = {
        sched_policy = SCHED_FIFO | SCHED_RR,
        sched_flags = 0,
        ..
    },
    flags = 0,
);
// Set the scheduling policy of a real-time thread
sched_setattr(
    pid,
    attr = {
        sched_policy = SCHED_FIFO | SCHED_RR,
        sched_flags = 0,
        ..
    },
    flags = 0,
);
```

Unsupported scheduling policies:
* `SCHED_DEADLINE`

Unsupported scheduling flags:
* `SCHED_FLAG_RESET_ON_FORK`
* `SCHED_FLAG_RECLAIM`
* `SCHED_FLAG_DL_OVERRUN`
* `SCHED_FLAG_UTIL_CLAMP_MIN`
* `SCHED_FLAG_UTIL_CLAMP_MAX`

## `wait4`

Supported functionality in SCML:

```c
// Wait until a specified child process undergoes a state change (termination, stopping and resumption)
wait4(
    pid, wstatus,
    options = WNOHANG | WSTOPPED | WCONTINUED | WNOWAIT,
    rusage
);
```

Ignored options:
* `WEXITED`
* `WNOTHREAD`
* `WALL`
* `WCLONE`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/wait4.2.html).

## `clone`

Supported functionality in SCML:

```c
signal_flags = SIGHUP | SIGINT | SIGQUIT | SIGILL |
               SIGTRAP | SIGABRT | SIGSTKFLT | SIGFPE |
               SIGKILL | SIGBUS | SIGSEGV | SIGXCPU |
               SIGPIPE | SIGALRM | SIGTERM | SIGUSR1 |
               SIGUSR2 | SIGCHLD | SIGPWR | SIGVTALRM |
               SIGPROF | SIGIO | SIGWINCH | SIGSTOP |
               SIGTSTP | SIGCONT | SIGTTIN | SIGTTOU |
               SIGURG | SIGXFSZ | SIGSYS | SIGRTMIN;

opt_flags =
    // Optional flags
    //
    // Share the parent's virtual memory
    CLONE_VM |
    // Share the parent's filesystem
    CLONE_FS |
    // Share the parent's file descriptor table
    CLONE_FILES |
    // Share the parent's signal handlers
    CLONE_SIGHAND |
    // Place child in the same thread group as parent
    CLONE_THREAD |
    // Share the parent's System V semaphore adjustments
    CLONE_SYSVSEM |
    // Suspend parent until the child exits or calls `execve`
    CLONE_VFORK |
    // Create a new mount namespace for the child
    CLONE_NEWNS |
    // Write child `TID` to parent's memory
    CLONE_PARENT_SETTID |
    // Allocate a `PID` file descriptor for the child
    CLONE_PIDFD |
    // Set thread-local storage for the child
    CLONE_SETTLS |
    // Write child `TID` to child's memory
    CLONE_CHILD_SETTID |
    // Clear child `TID` in child's memory on exit
    CLONE_CHILD_CLEARTID |
    // Make the child's parent the same as the caller's parent
    CLONE_PARENT;

// Create a thread or process
clone(
    fn, stack,
    flags = <opt_flags> | <signal_flags>,
    func_arg, ..
);
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/clone.2.html).