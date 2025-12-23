# Linux Compatibility

> "We don't break user space."
>
> --- Linus Torvalds

Asterinas is dedicated to maintaining compatibility with the Linux ABI,
ensuring that applications and administrative tools
designed for Linux can seamlessly operate within Asterinas.
While we prioritize compatibility,
it is important to note that Asterinas does not,
nor will it in the future,
support the loading of Linux kernel modules.

## System Calls

At the time of writing,
Asterinas supports over 230 Linux system calls for the x86-64 architecture,
which are summarized in the table below.

| Numbers | Names                  | Supported      | Flag Coverage |
| ------- | ---------------------- | -------------- | --- |
| 0       | read                   | ✅             | 💯 |
| 1       | write                  | ✅             | 💯 |
| 2       | open                   | ✅             | [⚠️](syscall-flag-coverage/file-and-directory-operations/#open-and-openat) |
| 3       | close                  | ✅             | 💯 |
| 4       | stat                   | ✅             | 💯 |
| 5       | fstat                  | ✅             | 💯 |
| 6       | lstat                  | ✅             | 💯 |
| 7       | poll                   | ✅             | [⚠️](syscall-flag-coverage/file-descriptor-and-io-control/#poll-and-ppoll) |
| 8       | lseek                  | ✅             | [⚠️](syscall-flag-coverage/file-and-directory-operations/#lseek) |
| 9       | mmap                   | ✅             | [⚠️](syscall-flag-coverage/memory-management/#mmap-and-munmap) |
| 10      | mprotect               | ✅             | [⚠️](syscall-flag-coverage/memory-management/#mprotect) |
| 11      | munmap                 | ✅             | 💯 |
| 12      | brk                    | ✅             | 💯 |
| 13      | rt_sigaction           | ✅             | [⚠️](syscall-flag-coverage/signals-and-timers/#rt_sigaction) |
| 14      | rt_sigprocmask         | ✅             | [⚠️](syscall-flag-coverage/signals-and-timers/#rt_sigprocmask) |
| 15      | rt_sigreturn           | ✅             | 💯 |
| 16      | ioctl                  | ✅             | [⚠️](syscall-flag-coverage/file-descriptor-and-io-control/#ioctl) |
| 17      | pread64                | ✅             | 💯 |
| 18      | pwrite64               | ✅             | 💯 |
| 19      | readv                  | ✅             | 💯 |
| 20      | writev                 | ✅             | 💯 |
| 21      | access                 | ✅             | 💯 |
| 22      | pipe                   | ✅             | 💯 |
| 23      | select                 | ✅             | 💯 |
| 24      | sched_yield            | ✅             | 💯 |
| 25      | mremap                 | ✅             | [⚠️](syscall-flag-coverage/memory-management/#mremap) |
| 26      | msync                  | ✅             | [⚠️](syscall-flag-coverage/memory-management/#msync) |
| 27      | mincore                | ❌             | N/A |
| 28      | madvise                | ✅             | [⚠️](syscall-flag-coverage/memory-management/#madvise) |
| 29      | shmget                 | ❌             | N/A |
| 30      | shmat                  | ❌             | N/A |
| 31      | shmctl                 | ❌             | N/A |
| 32      | dup                    | ✅             | 💯 |
| 33      | dup2                   | ✅             | 💯 |
| 34      | pause                  | ✅             | 💯 |
| 35      | nanosleep              | ✅             | 💯 |
| 36      | getitimer              | ✅             | 💯 |
| 37      | alarm                  | ✅             | 💯 |
| 38      | setitimer              | ✅             | 💯 |
| 39      | getpid                 | ✅             | 💯 |
| 40      | sendfile               | ✅             | 💯 |
| 41      | socket                 | ✅             | [⚠️](syscall-flag-coverage/networking-and-sockets/#socket) |
| 42      | connect                | ✅             | [⚠️](syscall-flag-coverage/networking-and-sockets/#connect) |
| 43      | accept                 | ✅             | [⚠️](syscall-flag-coverage/networking-and-sockets/#accept-and-accept4) |
| 44      | sendto                 | ✅             | [⚠️](syscall-flag-coverage/networking-and-sockets/#sendto-sendmsg-and-sendmmsg) |
| 45      | recvfrom               | ✅             | [⚠️](syscall-flag-coverage/networking-and-sockets/#recvfrom-and-recvmsg) |
| 46      | sendmsg                | ✅             | [⚠️](syscall-flag-coverage/networking-and-sockets/#sendto-sendmsg-and-sendmmsg) |
| 47      | recvmsg                | ✅             | [⚠️](syscall-flag-coverage/networking-and-sockets/#recvfrom-and-recvmsg) |
| 48      | shutdown               | ✅             | 💯 |
| 49      | bind                   | ✅             | [⚠️](syscall-flag-coverage/networking-and-sockets/#bind) |
| 50      | listen                 | ✅             | 💯 |
| 51      | getsockname            | ✅             | 💯 |
| 52      | getpeername            | ✅             | 💯 |
| 53      | socketpair             | ✅             | [⚠️](syscall-flag-coverage/networking-and-sockets/#socketpair) |
| 54      | setsockopt             | ✅             | [⚠️](syscall-flag-coverage/networking-and-sockets/#getsockopt-and-setsockopt) |
| 55      | getsockopt             | ✅             | [⚠️](syscall-flag-coverage/networking-and-sockets/#getsockopt-and-setsockopt) |
| 56      | clone                  | ✅             | [⚠️](syscall-flag-coverage/process-and-thread-management/#clone-and-clone3) |
| 57      | fork                   | ✅             | 💯 |
| 58      | vfork                  | ✅             | 💯 |
| 59      | execve                 | ✅             | 💯 |
| 60      | exit                   | ✅             | 💯 |
| 61      | wait4                  | ✅             | [⚠️](syscall-flag-coverage/process-and-thread-management/#wait4) |
| 62      | kill                   | ✅             | 💯 |
| 63      | uname                  | ✅             | 💯 |
| 64      | semget                 | ✅             | [⚠️](syscall-flag-coverage/inter-process-communication/#semget) |
| 65      | semop                  | ✅             | [⚠️](syscall-flag-coverage/inter-process-communication/#semop-and-semtimedop) |
| 66      | semctl                 | ✅             | [⚠️](syscall-flag-coverage/inter-process-communication/#semctl) |
| 67      | shmdt                  | ❌             | N/A |
| 68      | msgget                 | ❌             | N/A |
| 69      | msgsnd                 | ❌             | N/A |
| 70      | msgrcv                 | ❌             | N/A |
| 71      | msgctl                 | ❌             | N/A |
| 72      | fcntl                  | ✅             | [⚠️](syscall-flag-coverage/file-descriptor-and-io-control/#fcntl) |
| 73      | flock                  | ✅             | 💯 |
| 74      | fsync                  | ✅             | 💯 |
| 75      | fdatasync              | ✅             | 💯 |
| 76      | truncate               | ✅             | 💯 |
| 77      | ftruncate              | ✅             | 💯 |
| 78      | getdents               | ✅             | 💯 |
| 79      | getcwd                 | ✅             | 💯 |
| 80      | chdir                  | ✅             | 💯 |
| 81      | fchdir                 | ✅             | 💯 |
| 82      | rename                 | ✅             | 💯 |
| 83      | mkdir                  | ✅             | 💯 |
| 84      | rmdir                  | ✅             | 💯 |
| 85      | creat                  | ✅             | 💯 |
| 86      | link                   | ✅             | 💯 |
| 87      | unlink                 | ✅             | 💯 |
| 88      | symlink                | ✅             | 💯 |
| 89      | readlink               | ✅             | 💯 |
| 90      | chmod                  | ✅             | 💯 |
| 91      | fchmod                 | ✅             | 💯 |
| 92      | chown                  | ✅             | 💯 |
| 93      | fchown                 | ✅             | 💯 |
| 94      | lchown                 | ✅             | 💯 |
| 95      | umask                  | ✅             | 💯 |
| 96      | gettimeofday           | ✅             | 💯 |
| 97      | getrlimit              | ✅             | 💯 |
| 98      | getrusage              | ✅             | [⚠️](syscall-flag-coverage/system-information-and-misc/#getrusage) |
| 99      | sysinfo                | ✅             | 💯 |
| 100     | times                  | ❌             | N/A |
| 101     | ptrace                 | ❌             | N/A |
| 102     | getuid                 | ✅             | 💯 |
| 103     | syslog                 | ❌             | N/A |
| 104     | getgid                 | ✅             | 💯 |
| 105     | setuid                 | ✅             | 💯 |
| 106     | setgid                 | ✅             | 💯 |
| 107     | geteuid                | ✅             | 💯 |
| 108     | getegid                | ✅             | 💯 |
| 109     | setpgid                | ✅             | 💯 |
| 110     | getppid                | ✅             | 💯 |
| 111     | getpgrp                | ✅             | 💯 |
| 112     | setsid                 | ✅             | 💯 |
| 113     | setreuid               | ✅             | 💯 |
| 114     | setregid               | ✅             | 💯 |
| 115     | getgroups              | ✅             | 💯 |
| 116     | setgroups              | ✅             | 💯 |
| 117     | setresuid              | ✅             | 💯 |
| 118     | getresuid              | ✅             | 💯 |
| 119     | setresgid              | ✅             | 💯 |
| 120     | getresgid              | ✅             | 💯 |
| 121     | getpgid                | ✅             | 💯 |
| 122     | setfsuid               | ✅             | 💯 |
| 123     | setfsgid               | ✅             | 💯 |
| 124     | getsid                 | ✅             | 💯 |
| 125     | capget                 | ✅             | [⚠️](syscall-flag-coverage/namespaces-cgroups-and-security/#capget-and-capset) |
| 126     | capset                 | ✅             | [⚠️](syscall-flag-coverage/namespaces-cgroups-and-security/#capget-and-capset) |
| 127     | rt_sigpending          | ✅             | 💯 |
| 128     | rt_sigtimedwait        | ✅             | 💯 |
| 129     | rt_sigqueueinfo        | ❌             | N/A |
| 130     | rt_sigsuspend          | ✅             | 💯 |
| 131     | sigaltstack            | ✅             | 💯 |
| 132     | utime                  | ✅             | 💯 |
| 133     | mknod                  | ✅             | 💯 |
| 134     | uselib                 | ❌             | N/A |
| 135     | personality            | ❌             | N/A |
| 136     | ustat                  | ❌             | N/A |
| 137     | statfs                 | ✅             | 💯 |
| 138     | fstatfs                | ✅             | 💯 |
| 139     | sysfs                  | ❌             | N/A |
| 140     | getpriority            | ✅             | 💯 |
| 141     | setpriority            | ✅             | 💯 |
| 142     | sched_setparam         | ✅             | 💯 |
| 143     | sched_getparam         | ✅             | 💯 |
| 144     | sched_setscheduler     | ✅             | [⚠️](syscall-flag-coverage/process-and-thread-management/#sched_setscheduler) |
| 145     | sched_getscheduler     | ✅             | 💯 |
| 146     | sched_get_priority_max | ✅             | 💯 |
| 147     | sched_get_priority_min | ✅             | 💯 |
| 148     | sched_rr_get_interval  | ❌             | N/A |
| 149     | mlock                  | ❌             | N/A |
| 150     | munlock                | ❌             | N/A |
| 151     | mlockall               | ❌             | N/A |
| 152     | munlockall             | ❌             | N/A |
| 153     | vhangup                | ❌             | N/A |
| 154     | modify_ldt             | ❌             | N/A |
| 155     | pivot_root             | ❌             | N/A |
| 156     | _sysctl                | ❌             | N/A |
| 157     | prctl                  | ✅             | [⚠️](syscall-flag-coverage/namespaces-cgroups-and-security/#prctl) |
| 158     | arch_prctl             | ✅             | [⚠️](syscall-flag-coverage/system-information-and-misc/#arch_prctl) |
| 159     | adjtimex               | ❌             | N/A |
| 160     | setrlimit              | ✅             | 💯 |
| 161     | chroot                 | ✅             | 💯 |
| 162     | sync                   | ✅             | 💯 |
| 163     | acct                   | ❌             | N/A |
| 164     | settimeofday           | ❌             | N/A |
| 165     | mount                  | ✅             | [⚠️](syscall-flag-coverage/file-systems-and-mount-control/#mount) |
| 166     | umount2                | ✅             | [⚠️](syscall-flag-coverage/file-systems-and-mount-control/#umount-and-umount2) |
| 167     | swapon                 | ❌             | N/A |
| 168     | swapoff                | ❌             | N/A |
| 169     | reboot                 | ✅             | [⚠️](syscall-flag-coverage/system-information-and-misc/#reboot) |
| 170     | sethostname            | ✅             | 💯 |
| 171     | setdomainname          | ✅             | 💯 |
| 172     | iopl                   | ❌             | N/A |
| 173     | ioperm                 | ❌             | N/A |
| 174     | create_module          | ❌             | N/A |
| 175     | init_module            | ❌             | N/A |
| 176     | delete_module          | ❌             | N/A |
| 177     | get_kernel_syms        | ❌             | N/A |
| 178     | query_module           | ❌             | N/A |
| 179     | quotactl               | ❌             | N/A |
| 180     | nfsservctl             | ❌             | N/A |
| 181     | getpmsg                | ❌             | N/A |
| 182     | putpmsg                | ❌             | N/A |
| 183     | afs_syscall            | ❌             | N/A |
| 184     | tuxcall                | ❌             | N/A |
| 185     | security               | ❌             | N/A |
| 186     | gettid                 | ✅             | 💯 |
| 187     | readahead              | ❌             | N/A |
| 188     | setxattr               | ✅             | 💯 |
| 189     | lsetxattr              | ✅             | 💯 |
| 190     | fsetxattr              | ✅             | 💯 |
| 191     | getxattr               | ✅             | 💯 |
| 192     | lgetxattr              | ✅             | 💯 |
| 193     | fgetxattr              | ✅             | 💯 |
| 194     | listxattr              | ✅             | 💯 |
| 195     | llistxattr             | ✅             | 💯 |
| 196     | flistxattr             | ✅             | 💯 |
| 197     | removexattr            | ✅             | 💯 |
| 198     | lremovexattr           | ✅             | 💯 |
| 199     | fremovexattr           | ✅             | 💯 |
| 200     | tkill                  | ❌             | N/A |
| 201     | time                   | ✅             | 💯 |
| 202     | futex                  | ✅             | [⚠️](syscall-flag-coverage/inter-process-communication/#futex) |
| 203     | sched_setaffinity      | ✅             | 💯 |
| 204     | sched_getaffinity      | ✅             | 💯 |
| 205     | set_thread_area        | ❌             | N/A |
| 206     | io_setup               | ❌             | N/A |
| 207     | io_destroy             | ❌             | N/A |
| 208     | io_getevents           | ❌             | N/A |
| 209     | io_submit              | ❌             | N/A |
| 210     | io_cancel              | ❌             | N/A |
| 211     | get_thread_area        | ❌             | N/A |
| 212     | lookup_dcookie         | ❌             | N/A |
| 213     | epoll_create           | ✅             | 💯 |
| 214     | epoll_ctl_old          | ❌             | N/A |
| 215     | epoll_wait_old         | ❌             | N/A |
| 216     | remap_file_pages       | ❌             | N/A |
| 217     | getdents64             | ✅             | 💯 |
| 218     | set_tid_address        | ✅             | 💯 |
| 219     | restart_syscall        | ❌             | N/A |
| 220     | semtimedop             | ✅             | [⚠️](syscall-flag-coverage/inter-process-communication/#semop-and-semtimedop) |
| 221     | fadvise64              | ✅             | [⚠️](syscall-flag-coverage/file-descriptor-and-io-control/#fadvise64) |
| 222     | timer_create           | ✅             | [⚠️](syscall-flag-coverage/signals-and-timers/#timer_create) |
| 223     | timer_settime          | ✅             | 💯 |
| 224     | timer_gettime          | ✅             | 💯 |
| 225     | timer_getoverrun       | ❌             | N/A |
| 226     | timer_delete           | ✅             | 💯 |
| 227     | clock_settime          | ❌             | N/A |
| 228     | clock_gettime          | ✅             | [⚠️](syscall-flag-coverage/system-information-and-misc/#clock_gettime) |
| 229     | clock_getres           | ❌             | N/A |
| 230     | clock_nanosleep        | ✅             | [⚠️](syscall-flag-coverage/system-information-and-misc/#clock_nanosleep) |
| 231     | exit_group             | ✅             | 💯 |
| 232     | epoll_wait             | ✅             | 💯 |
| 233     | epoll_ctl              | ✅             | [⚠️](syscall-flag-coverage/file-descriptor-and-io-control/#epoll_ctl) |
| 234     | tgkill                 | ✅             | 💯 |
| 235     | utimes                 | ✅             | 💯 |
| 236     | vserver                | ❌             | N/A |
| 237     | mbind                  | ❌             | N/A |
| 238     | set_mempolicy          | ❌             | N/A |
| 239     | get_mempolicy          | ❌             | N/A |
| 240     | mq_open                | ❌             | N/A |
| 241     | mq_unlink              | ❌             | N/A |
| 242     | mq_timedsend           | ❌             | N/A |
| 243     | mq_timedreceive        | ❌             | N/A |
| 244     | mq_notify              | ❌             | N/A |
| 245     | mq_getsetattr          | ❌             | N/A |
| 246     | kexec_load             | ❌             | N/A |
| 247     | waitid                 | ✅             | [⚠️](syscall-flag-coverage/process-and-thread-management/#waitid) |
| 248     | add_key                | ❌             | N/A |
| 249     | request_key            | ❌             | N/A |
| 250     | keyctl                 | ❌             | N/A |
| 251     | ioprio_set             | ✅             | [⚠️](syscall-flag-coverage/file-descriptor-and-io-control/#ioprio_set-and-ioprio_get) |
| 252     | ioprio_get             | ✅             | [⚠️](syscall-flag-coverage/file-descriptor-and-io-control/#ioprio_set-and-ioprio_get) |
| 253     | inotify_init           | ✅             | 💯 |
| 254     | inotify_add_watch      | ✅             | [⚠️](syscall-flag-coverage/file-systems-and-mount-control/#inotify_add_watch) |
| 255     | inotify_rm_watch       | ✅             | 💯 |
| 256     | migrate_pages          | ❌             | N/A |
| 257     | openat                 | ✅             | [⚠️](syscall-flag-coverage/file-and-directory-operations/#open-and-openat) |
| 258     | mkdirat                | ✅             | 💯 |
| 259     | mknodat                | ✅             | 💯 |
| 260     | fchownat               | ✅             | 💯 |
| 261     | futimesat              | ✅             | 💯 |
| 262     | newfstatat             | ✅             | [⚠️](syscall-flag-coverage/file-and-directory-operations/#newfstatat) |
| 263     | unlinkat               | ✅             | 💯 |
| 264     | renameat               | ✅             | 💯 |
| 265     | linkat                 | ✅             | 💯 |
| 266     | symlinkat              | ✅             | 💯 |
| 267     | readlinkat             | ✅             | 💯 |
| 268     | fchmodat               | ✅             | 💯 |
| 269     | faccessat              | ✅             | 💯 |
| 270     | pselect6               | ✅             | 💯 |
| 271     | ppoll                  | ✅             | [⚠️](syscall-flag-coverage/file-descriptor-and-io-control/#poll-and-ppoll) |
| 272     | unshare                | ✅             | [⚠️](syscall-flag-coverage/namespaces-cgroups-and-security/#unshare) |
| 273     | set_robust_list        | ✅             | 💯 |
| 274     | get_robust_list        | ❌             | N/A |
| 275     | splice                 | ❌             | N/A |
| 276     | tee                    | ❌             | N/A |
| 277     | sync_file_range        | ❌             | N/A |
| 278     | vmsplice               | ❌             | N/A |
| 279     | move_pages             | ❌             | N/A |
| 280     | utimensat              | ✅             | [⚠️](syscall-flag-coverage/file-and-directory-operations/#utimensat) |
| 281     | epoll_pwait            | ✅             | 💯 |
| 282     | signalfd               | ✅             | 💯 |
| 283     | timerfd_create         | ✅             | [⚠️](syscall-flag-coverage/signals-and-timers/#timerfd_create) |
| 284     | eventfd                | ✅             | 💯 |
| 285     | fallocate              | ✅             | [⚠️](syscall-flag-coverage/file-and-directory-operations/#fallocate) |
| 286     | timerfd_settime        | ✅             | [⚠️](syscall-flag-coverage/signals-and-timers/#timerfd_settime) |
| 287     | timerfd_gettime        | ✅             | 💯 |
| 288     | accept4                | ✅             | [⚠️](syscall-flag-coverage/networking-and-sockets/#accept-and-accept4) |
| 289     | signalfd4              | ✅             | 💯 |
| 290     | eventfd2               | ✅             | [⚠️](syscall-flag-coverage/file-descriptor-and-io-control/#eventfd-and-eventfd2) |
| 291     | epoll_create1          | ✅             | 💯 |
| 292     | dup3                   | ✅             | 💯 |
| 293     | pipe2                  | ✅             | [⚠️](syscall-flag-coverage/file-descriptor-and-io-control/#pipe-and-pipe2) |
| 294     | inotify_init1          | ✅             | [⚠️](syscall-flag-coverage/file-systems-and-mount-control/#inotify_init-and-inotify_init1) |
| 295     | preadv                 | ✅             | 💯 |
| 296     | pwritev                | ✅             | 💯 |
| 297     | rt_tgsigqueueinfo      | ❌             | N/A |
| 298     | perf_event_open        | ❌             | N/A |
| 299     | recvmmsg               | ❌             | N/A |
| 300     | fanotify_init          | ❌             | N/A |
| 301     | fanotify_mark          | ❌             | N/A |
| 302     | prlimit64              | ✅             | 💯 |
| 303     | name_to_handle_at      | ❌             | N/A |
| 304     | open_by_handle_at      | ❌             | N/A |
| 305     | clock_adjtime          | ❌             | N/A |
| 306     | syncfs                 | ✅             | 💯 |
| 307     | sendmmsg               | ✅             | [⚠️](syscall-flag-coverage/networking-and-sockets/#sendto-sendmsg-and-sendmmsg) |
| 308     | setns                  | ✅             | [⚠️](syscall-flag-coverage/namespaces-cgroups-and-security/#setns) |
| 309     | getcpu                 | ✅             | 💯 |
| 310     | process_vm_readv       | ❌             | N/A |
| 311     | process_vm_writev      | ❌             | N/A |
| 312     | kcmp                   | ❌             | N/A |
| 313     | finit_module           | ❌             | N/A |
| 314     | sched_setattr          | ✅             | [⚠️](syscall-flag-coverage/process-and-thread-management/#sched_getattr-and-sched_setattr) |
| 315     | sched_getattr          | ✅             | [⚠️](syscall-flag-coverage/process-and-thread-management/#sched_getattr-and-sched_setattr) |
| 316     | renameat2              | ✅             | [⚠️](syscall-flag-coverage/file-and-directory-operations/#renameat2) |
| 318     | getrandom              | ✅             | [⚠️](syscall-flag-coverage/system-information-and-misc/#getrandom) |
| 319     | memfd_create           | ✅             | [⚠️](syscall-flag-coverage/file-descriptor-and-io-control/#memfd_create) |
| 322     | execveat               | ✅             | 💯 |
| 327     | preadv2                | ✅             | [⚠️](syscall-flag-coverage/file-and-directory-operations/#preadv2-and-pwritev2) |
| 328     | pwritev2               | ✅             | [⚠️](syscall-flag-coverage/file-and-directory-operations/#preadv2-and-pwritev2) |
| 332     | statx                  | ✅             | [⚠️](syscall-flag-coverage/file-and-directory-operations/#statx) |
| 434     | pidfd_open             | ✅             | 💯 |
| 435     | clone3                 | ✅             | [⚠️](syscall-flag-coverage/process-and-thread-management/#clone-and-clone3) |
| 436     | close_range            | ✅             | 💯 |
| 439     | faccessat2             | ✅             | [⚠️](syscall-flag-coverage/file-and-directory-operations/#faccessat2) |
| 441     | epoll_pwait2           | ✅             | 💯 |
| 452     | fchmodat2              | ✅             | 💯 |

- Supported:
    - ✅ = syscall supported
    - ❌ = not supported

- Flag Coverage:
    - 💯 = all flags/commands/modes are supported
    - ⚠️ = syscall works, but some flags/modes are not implemented
    - ❓ = implementation exists, but we have not audited its coverage yet
    - N/A = not applicable (e.g., syscall not supported)

Most of these system calls (or their variants) are also supported
for the RISC-V and LoongArch architectures.

## File Systems

Here is the list of supported file systems:
* Devfs
* Devpts
* Ext2
* Procfs
* Ramfs

## Sockets

Here is the list of supported socket types:
* TCP sockets over IPv4
* UDP sockets over IPv4
* Unix sockets

## vDSO

Here is the list of supported symbols in vDSO:
* `__vdso_clock_gettime`
* `__vdso_gettimeofday`
* `__vdso_time`

## Boot Protocols

Here is the list of supported boot protocols:
* [Multiboot](https://www.gnu.org/software/grub/manual/multiboot/multiboot.html)
* [Multiboot2](https://www.gnu.org/software/grub/manual/multiboot2/multiboot.html)
* [Linux 32-bit boot protocol](https://www.kernel.org/doc/html/v5.4/x86/boot.html#bit-boot-protocol)
* [Linux EFI handover](https://www.kernel.org/doc/html/v5.4/x86/boot.html#efi-handover-protocol)
