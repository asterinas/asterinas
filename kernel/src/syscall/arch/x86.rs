// SPDX-License-Identifier: MPL-2.0

use crate::syscall::{
    accept::{sys_accept, sys_accept4},
    access::{sys_access, sys_faccessat},
    alarm::sys_alarm,
    arch_prctl::sys_arch_prctl,
    bind::sys_bind,
    brk::sys_brk,
    capget::sys_capget,
    capset::sys_capset,
    chdir::{sys_chdir, sys_fchdir},
    chmod::{sys_chmod, sys_fchmod, sys_fchmodat},
    chown::{sys_chown, sys_fchown, sys_fchownat, sys_lchown},
    chroot::sys_chroot,
    clock_gettime::sys_clock_gettime,
    clone::{sys_clone, sys_clone3},
    close::sys_close,
    connect::sys_connect,
    dup::{sys_dup, sys_dup2, sys_dup3},
    epoll::{sys_epoll_create, sys_epoll_create1, sys_epoll_ctl, sys_epoll_pwait, sys_epoll_wait},
    eventfd::{sys_eventfd, sys_eventfd2},
    execve::{sys_execve, sys_execveat},
    exit::sys_exit,
    exit_group::sys_exit_group,
    fallocate::sys_fallocate,
    fcntl::sys_fcntl,
    flock::sys_flock,
    fork::sys_fork,
    fsync::{sys_fdatasync, sys_fsync},
    futex::sys_futex,
    getcwd::sys_getcwd,
    getdents64::{sys_getdents, sys_getdents64},
    getegid::sys_getegid,
    geteuid::sys_geteuid,
    getgid::sys_getgid,
    getgroups::sys_getgroups,
    getpeername::sys_getpeername,
    getpgid::sys_getpgid,
    getpgrp::sys_getpgrp,
    getpid::sys_getpid,
    getppid::sys_getppid,
    getrandom::sys_getrandom,
    getresgid::sys_getresgid,
    getresuid::sys_getresuid,
    getrusage::sys_getrusage,
    getsid::sys_getsid,
    getsockname::sys_getsockname,
    getsockopt::sys_getsockopt,
    gettid::sys_gettid,
    gettimeofday::sys_gettimeofday,
    getuid::sys_getuid,
    impl_syscall_nums_and_dispatch_fn,
    ioctl::sys_ioctl,
    kill::sys_kill,
    link::{sys_link, sys_linkat},
    listen::sys_listen,
    lseek::sys_lseek,
    madvise::sys_madvise,
    mkdir::{sys_mkdir, sys_mkdirat},
    mknod::{sys_mknod, sys_mknodat},
    mmap::sys_mmap,
    mount::sys_mount,
    mprotect::sys_mprotect,
    msync::sys_msync,
    munmap::sys_munmap,
    nanosleep::{sys_clock_nanosleep, sys_nanosleep},
    open::{sys_creat, sys_open, sys_openat},
    pause::sys_pause,
    pipe::{sys_pipe, sys_pipe2},
    poll::sys_poll,
    prctl::sys_prctl,
    pread64::sys_pread64,
    preadv::{sys_preadv, sys_preadv2, sys_readv},
    prlimit64::{sys_getrlimit, sys_prlimit64, sys_setrlimit},
    pselect6::sys_pselect6,
    pwrite64::sys_pwrite64,
    pwritev::{sys_pwritev, sys_pwritev2, sys_writev},
    read::sys_read,
    readlink::{sys_readlink, sys_readlinkat},
    recvfrom::sys_recvfrom,
    recvmsg::sys_recvmsg,
    rename::{sys_rename, sys_renameat},
    rmdir::sys_rmdir,
    rt_sigaction::sys_rt_sigaction,
    rt_sigpending::sys_rt_sigpending,
    rt_sigprocmask::sys_rt_sigprocmask,
    rt_sigreturn::sys_rt_sigreturn,
    rt_sigsuspend::sys_rt_sigsuspend,
    sched_affinity::{sys_sched_getaffinity, sys_sched_setaffinity},
    sched_yield::sys_sched_yield,
    select::sys_select,
    semctl::sys_semctl,
    semget::sys_semget,
    semop::{sys_semop, sys_semtimedop},
    sendfile::sys_sendfile,
    sendmsg::sys_sendmsg,
    sendto::sys_sendto,
    set_get_priority::{sys_get_priority, sys_set_priority},
    set_robust_list::sys_set_robust_list,
    set_tid_address::sys_set_tid_address,
    setfsgid::sys_setfsgid,
    setfsuid::sys_setfsuid,
    setgid::sys_setgid,
    setgroups::sys_setgroups,
    setitimer::{sys_getitimer, sys_setitimer},
    setpgid::sys_setpgid,
    setregid::sys_setregid,
    setresgid::sys_setresgid,
    setresuid::sys_setresuid,
    setreuid::sys_setreuid,
    setsid::sys_setsid,
    setsockopt::sys_setsockopt,
    setuid::sys_setuid,
    shutdown::sys_shutdown,
    sigaltstack::sys_sigaltstack,
    socket::sys_socket,
    socketpair::sys_socketpair,
    stat::{sys_fstat, sys_fstatat, sys_lstat, sys_stat},
    statfs::{sys_fstatfs, sys_statfs},
    symlink::{sys_symlink, sys_symlinkat},
    sync::sys_sync,
    tgkill::sys_tgkill,
    time::sys_time,
    timer_create::{sys_timer_create, sys_timer_delete},
    timer_settime::{sys_timer_gettime, sys_timer_settime},
    truncate::{sys_ftruncate, sys_truncate},
    umask::sys_umask,
    umount::sys_umount,
    uname::sys_uname,
    unlink::{sys_unlink, sys_unlinkat},
    utimens::{sys_futimesat, sys_utime, sys_utimensat, sys_utimes},
    wait4::sys_wait4,
    waitid::sys_waitid,
    write::sys_write,
};

impl_syscall_nums_and_dispatch_fn! {
    SYS_READ = 0               => sys_read(args[..3]);
    SYS_WRITE = 1              => sys_write(args[..3]);
    SYS_OPEN = 2               => sys_open(args[..3]);
    SYS_CLOSE = 3              => sys_close(args[..1]);
    SYS_STAT = 4               => sys_stat(args[..2]);
    SYS_FSTAT = 5              => sys_fstat(args[..2]);
    SYS_LSTAT = 6              => sys_lstat(args[..2]);
    SYS_POLL = 7               => sys_poll(args[..3]);
    SYS_LSEEK = 8              => sys_lseek(args[..3]);
    SYS_MMAP = 9               => sys_mmap(args[..6]);
    SYS_MPROTECT = 10          => sys_mprotect(args[..3]);
    SYS_MUNMAP = 11            => sys_munmap(args[..2]);
    SYS_BRK = 12               => sys_brk(args[..1]);
    SYS_RT_SIGACTION = 13      => sys_rt_sigaction(args[..4]);
    SYS_RT_SIGPROCMASK = 14    => sys_rt_sigprocmask(args[..4]);
    SYS_RT_SIGRETURN = 15      => sys_rt_sigreturn(args[..0], &mut user_ctx);
    SYS_IOCTL = 16             => sys_ioctl(args[..3]);
    SYS_PREAD64 = 17           => sys_pread64(args[..4]);
    SYS_PWRITE64 = 18          => sys_pwrite64(args[..4]);
    SYS_READV = 19             => sys_readv(args[..3]);
    SYS_WRITEV = 20            => sys_writev(args[..3]);
    SYS_ACCESS = 21            => sys_access(args[..2]);
    SYS_PIPE = 22              => sys_pipe(args[..1]);
    SYS_SELECT = 23            => sys_select(args[..5]);
    SYS_MSYNC = 26             => sys_msync(args[..3]);
    SYS_SCHED_YIELD = 24       => sys_sched_yield(args[..0]);
    SYS_MADVISE = 28           => sys_madvise(args[..3]);
    SYS_DUP = 32               => sys_dup(args[..1]);
    SYS_DUP2 = 33              => sys_dup2(args[..2]);
    SYS_PAUSE = 34             => sys_pause(args[..0]);
    SYS_NANOSLEEP = 35         => sys_nanosleep(args[..2]);
    SYS_GETITIMER = 36         => sys_getitimer(args[..2]);
    SYS_ALARM = 37             => sys_alarm(args[..1]);
    SYS_SETITIMER = 38         => sys_setitimer(args[..3]);
    SYS_GETPID = 39            => sys_getpid(args[..0]);
    SYS_SENDFILE = 40          => sys_sendfile(args[..4]);
    SYS_SOCKET = 41            => sys_socket(args[..3]);
    SYS_CONNECT = 42           => sys_connect(args[..3]);
    SYS_ACCEPT = 43            => sys_accept(args[..3]);
    SYS_SENDTO = 44            => sys_sendto(args[..6]);
    SYS_RECVFROM = 45          => sys_recvfrom(args[..6]);
    SYS_SENDMSG = 46           => sys_sendmsg(args[..3]);
    SYS_RECVMSG = 47           => sys_recvmsg(args[..3]);
    SYS_SHUTDOWN = 48          => sys_shutdown(args[..2]);
    SYS_BIND = 49              => sys_bind(args[..3]);
    SYS_LISTEN = 50            => sys_listen(args[..2]);
    SYS_GETSOCKNAME = 51       => sys_getsockname(args[..3]);
    SYS_GETPEERNAME = 52       => sys_getpeername(args[..3]);
    SYS_SOCKETPAIR = 53        => sys_socketpair(args[..4]);
    SYS_SETSOCKOPT = 54        => sys_setsockopt(args[..5]);
    SYS_GETSOCKOPT = 55        => sys_getsockopt(args[..5]);
    SYS_CLONE = 56             => sys_clone(args[..5], &user_ctx);
    SYS_FORK = 57              => sys_fork(args[..0], &user_ctx);
    SYS_EXECVE = 59            => sys_execve(args[..3], &mut user_ctx);
    SYS_EXIT = 60              => sys_exit(args[..1]);
    SYS_WAIT4 = 61             => sys_wait4(args[..4]);
    SYS_KILL = 62              => sys_kill(args[..2]);
    SYS_UNAME = 63             => sys_uname(args[..1]);
    SYS_SEMGET = 64            => sys_semget(args[..3]);
    SYS_SEMOP = 65             => sys_semop(args[..3]);
    SYS_SEMCTL = 66            => sys_semctl(args[..4]);
    SYS_FCNTL = 72             => sys_fcntl(args[..3]);
    SYS_FLOCK = 73             => sys_flock(args[..2]);
    SYS_FSYNC = 74             => sys_fsync(args[..1]);
    SYS_FDATASYNC = 75         => sys_fdatasync(args[..1]);
    SYS_TRUNCATE = 76          => sys_truncate(args[..2]);
    SYS_FTRUNCATE = 77         => sys_ftruncate(args[..2]);
    SYS_GETDENTS = 78          => sys_getdents(args[..3]);
    SYS_GETCWD = 79            => sys_getcwd(args[..2]);
    SYS_CHDIR = 80             => sys_chdir(args[..1]);
    SYS_FCHDIR = 81            => sys_fchdir(args[..1]);
    SYS_RENAME = 82            => sys_rename(args[..2]);
    SYS_MKDIR = 83             => sys_mkdir(args[..2]);
    SYS_RMDIR = 84             => sys_rmdir(args[..1]);
    SYS_CREAT = 85             => sys_creat(args[..2]);
    SYS_LINK = 86              => sys_link(args[..2]);
    SYS_UNLINK = 87            => sys_unlink(args[..1]);
    SYS_SYMLINK = 88           => sys_symlink(args[..2]);
    SYS_READLINK = 89          => sys_readlink(args[..3]);
    SYS_CHMOD = 90             => sys_chmod(args[..2]);
    SYS_FCHMOD = 91            => sys_fchmod(args[..2]);
    SYS_CHOWN = 92             => sys_chown(args[..3]);
    SYS_FCHOWN = 93            => sys_fchown(args[..3]);
    SYS_LCHOWN = 94            => sys_lchown(args[..3]);
    SYS_UMASK = 95             => sys_umask(args[..1]);
    SYS_GETTIMEOFDAY = 96      => sys_gettimeofday(args[..1]);
    SYS_GETRLIMIT = 97         => sys_getrlimit(args[..2]);
    SYS_GETRUSAGE = 98         => sys_getrusage(args[..2]);
    SYS_GETUID = 102           => sys_getuid(args[..0]);
    SYS_GETGID = 104           => sys_getgid(args[..0]);
    SYS_SETUID = 105           => sys_setuid(args[..1]);
    SYS_SETGID = 106           => sys_setgid(args[..1]);
    SYS_GETEUID = 107          => sys_geteuid(args[..0]);
    SYS_GETEGID = 108          => sys_getegid(args[..0]);
    SYS_SETPGID = 109          => sys_setpgid(args[..2]);
    SYS_GETPPID = 110          => sys_getppid(args[..0]);
    SYS_GETPGRP = 111          => sys_getpgrp(args[..0]);
    SYS_SETSID = 112           => sys_setsid(args[..0]);
    SYS_SETREUID = 113         => sys_setreuid(args[..2]);
    SYS_SETREGID = 114         => sys_setregid(args[..2]);
    SYS_GETGROUPS = 115        => sys_getgroups(args[..2]);
    SYS_SETGROUPS = 116        => sys_setgroups(args[..2]);
    SYS_SETRESUID = 117        => sys_setresuid(args[..3]);
    SYS_GETRESUID = 118        => sys_getresuid(args[..3]);
    SYS_SETRESGID = 119        => sys_setresgid(args[..3]);
    SYS_GETRESGID = 120        => sys_getresgid(args[..3]);
    SYS_GETPGID = 121          => sys_getpgid(args[..1]);
    SYS_SETFSUID = 122         => sys_setfsuid(args[..1]);
    SYS_SETFSGID = 123         => sys_setfsgid(args[..1]);
    SYS_GETSID = 124           => sys_getsid(args[..1]);
    SYS_CAPGET = 125           => sys_capget(args[..2]);
    SYS_CAPSET = 126           => sys_capset(args[..2]);
    SYS_RT_SIGPENDING = 127    => sys_rt_sigpending(args[..2]);
    SYS_RT_SIGSUSPEND = 130    => sys_rt_sigsuspend(args[..2]);
    SYS_SIGALTSTACK = 131      => sys_sigaltstack(args[..2]);
    SYS_UTIME = 132            => sys_utime(args[..2]);
    SYS_MKNOD = 133            => sys_mknod(args[..3]);
    SYS_STATFS = 137           => sys_statfs(args[..2]);
    SYS_FSTATFS = 138          => sys_fstatfs(args[..2]);
    SYS_GET_PRIORITY = 140     => sys_get_priority(args[..2]);
    SYS_SET_PRIORITY = 141     => sys_set_priority(args[..3]);
    SYS_PRCTL = 157            => sys_prctl(args[..5]);
    SYS_ARCH_PRCTL = 158       => sys_arch_prctl(args[..2], &mut user_ctx);
    SYS_SETRLIMIT = 160        => sys_setrlimit(args[..2]);
    SYS_CHROOT = 161           => sys_chroot(args[..1]);
    SYS_SYNC = 162             => sys_sync(args[..0]);
    SYS_MOUNT = 165            => sys_mount(args[..5]);
    SYS_UMOUNT2 = 166           => sys_umount(args[..2]);
    SYS_GETTID = 186           => sys_gettid(args[..0]);
    SYS_TIME = 201             => sys_time(args[..1]);
    SYS_FUTEX = 202            => sys_futex(args[..6]);
    SYS_SCHED_SETAFFINITY = 203 => sys_sched_setaffinity(args[..3]);
    SYS_SCHED_GETAFFINITY = 204 => sys_sched_getaffinity(args[..3]);
    SYS_EPOLL_CREATE = 213     => sys_epoll_create(args[..1]);
    SYS_GETDENTS64 = 217       => sys_getdents64(args[..3]);
    SYS_SET_TID_ADDRESS = 218  => sys_set_tid_address(args[..1]);
    SYS_SEMTIMEDOP = 220       => sys_semtimedop(args[..4]);
    SYS_TIMER_CREATE = 222     => sys_timer_create(args[..3]);
    SYS_TIMER_SETTIME = 223    => sys_timer_settime(args[..4]);
    SYS_TIMER_GETTIME = 224    => sys_timer_gettime(args[..2]);
    SYS_TIMER_DELETE = 226     => sys_timer_delete(args[..1]);
    SYS_CLOCK_GETTIME = 228    => sys_clock_gettime(args[..2]);
    SYS_CLOCK_NANOSLEEP = 230  => sys_clock_nanosleep(args[..4]);
    SYS_EXIT_GROUP = 231       => sys_exit_group(args[..1]);
    SYS_EPOLL_WAIT = 232       => sys_epoll_wait(args[..4]);
    SYS_EPOLL_CTL = 233        => sys_epoll_ctl(args[..4]);
    SYS_TGKILL = 234           => sys_tgkill(args[..3]);
    SYS_UTIMES = 235           => sys_utimes(args[..2]);
    SYS_WAITID = 247           => sys_waitid(args[..5]);
    SYS_OPENAT = 257           => sys_openat(args[..4]);
    SYS_MKDIRAT = 258          => sys_mkdirat(args[..3]);
    SYS_MKNODAT = 259          => sys_mknodat(args[..4]);
    SYS_FCHOWNAT = 260         => sys_fchownat(args[..5]);
    SYS_FUTIMESAT = 261        => sys_futimesat(args[..3]);
    SYS_FSTATAT = 262          => sys_fstatat(args[..4]);
    SYS_UNLINKAT = 263         => sys_unlinkat(args[..3]);
    SYS_RENAMEAT = 264         => sys_renameat(args[..4]);
    SYS_LINKAT = 265           => sys_linkat(args[..5]);
    SYS_SYMLINKAT = 266        => sys_symlinkat(args[..3]);
    SYS_READLINKAT = 267       => sys_readlinkat(args[..4]);
    SYS_FCHMODAT = 268         => sys_fchmodat(args[..3]);
    SYS_FACCESSAT = 269        => sys_faccessat(args[..3]);
    SYS_PSELECT6 = 270         => sys_pselect6(args[..6]);
    SYS_SET_ROBUST_LIST = 273  => sys_set_robust_list(args[..2]);
    SYS_UTIMENSAT = 280        => sys_utimensat(args[..4]);
    SYS_EPOLL_PWAIT = 281      => sys_epoll_pwait(args[..6]);
    SYS_EVENTFD = 284          => sys_eventfd(args[..1]);
    SYS_FALLOCATE = 285        => sys_fallocate(args[..4]);
    SYS_ACCEPT4 = 288          => sys_accept4(args[..4]);
    SYS_EVENTFD2 = 290         => sys_eventfd2(args[..2]);
    SYS_EPOLL_CREATE1 = 291    => sys_epoll_create1(args[..1]);
    SYS_DUP3 = 292             => sys_dup3(args[..3]);
    SYS_PIPE2 = 293            => sys_pipe2(args[..2]);
    SYS_PREADV = 295           => sys_preadv(args[..4]);
    SYS_PWRITEV = 296          => sys_pwritev(args[..4]);
    SYS_PRLIMIT64 = 302        => sys_prlimit64(args[..4]);
    SYS_GETRANDOM = 318        => sys_getrandom(args[..3]);
    SYS_EXECVEAT = 322         => sys_execveat(args[..5], &mut user_ctx);
    SYS_PREADV2 = 327          => sys_preadv2(args[..5]);
    SYS_PWRITEV2 = 328         => sys_pwritev2(args[..5]);
    SYS_CLONE3 = 435           => sys_clone3(args[..2], &user_ctx);
}
