// SPDX-License-Identifier: MPL-2.0

use crate::syscall::{
    accept::{sys_accept, sys_accept4},
    access::sys_faccessat,
    bind::sys_bind,
    brk::sys_brk,
    capget::sys_capget,
    capset::sys_capset,
    chdir::{sys_chdir, sys_fchdir},
    chmod::{sys_fchmod, sys_fchmodat},
    chown::{sys_fchown, sys_fchownat},
    chroot::sys_chroot,
    clock_gettime::sys_clock_gettime,
    clone::{sys_clone, sys_clone3},
    close::sys_close,
    connect::sys_connect,
    dup::{sys_dup, sys_dup3},
    epoll::{sys_epoll_create1, sys_epoll_ctl, sys_epoll_pwait},
    eventfd::sys_eventfd2,
    execve::{sys_execve, sys_execveat},
    exit::sys_exit,
    exit_group::sys_exit_group,
    fallocate::sys_fallocate,
    fcntl::sys_fcntl,
    flock::sys_flock,
    fsync::{sys_fdatasync, sys_fsync},
    futex::sys_futex,
    getcwd::sys_getcwd,
    getdents64::sys_getdents64,
    getegid::sys_getegid,
    geteuid::sys_geteuid,
    getgid::sys_getgid,
    getgroups::sys_getgroups,
    getpeername::sys_getpeername,
    getpgid::sys_getpgid,
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
    link::sys_linkat,
    listen::sys_listen,
    lseek::sys_lseek,
    madvise::sys_madvise,
    mkdir::sys_mkdirat,
    mknod::sys_mknodat,
    mmap::sys_mmap,
    mount::sys_mount,
    mprotect::sys_mprotect,
    msync::sys_msync,
    munmap::sys_munmap,
    nanosleep::{sys_clock_nanosleep, sys_nanosleep},
    open::sys_openat,
    pipe::sys_pipe2,
    prctl::sys_prctl,
    pread64::sys_pread64,
    preadv::{sys_preadv, sys_preadv2, sys_readv},
    prlimit64::{sys_getrlimit, sys_prlimit64, sys_setrlimit},
    pselect6::sys_pselect6,
    pwrite64::sys_pwrite64,
    pwritev::{sys_pwritev, sys_pwritev2, sys_writev},
    read::sys_read,
    readlink::sys_readlinkat,
    recvfrom::sys_recvfrom,
    recvmsg::sys_recvmsg,
    rename::sys_renameat,
    rt_sigaction::sys_rt_sigaction,
    rt_sigpending::sys_rt_sigpending,
    rt_sigprocmask::sys_rt_sigprocmask,
    rt_sigsuspend::sys_rt_sigsuspend,
    sched_affinity::{sys_sched_getaffinity, sys_sched_setaffinity},
    sched_yield::sys_sched_yield,
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
    stat::{sys_fstat, sys_fstatat},
    statfs::{sys_fstatfs, sys_statfs},
    symlink::sys_symlinkat,
    sync::sys_sync,
    tgkill::sys_tgkill,
    timer_create::{sys_timer_create, sys_timer_delete},
    timer_settime::{sys_timer_gettime, sys_timer_settime},
    truncate::{sys_ftruncate, sys_truncate},
    umask::sys_umask,
    umount::sys_umount,
    uname::sys_uname,
    unlink::sys_unlinkat,
    utimens::sys_utimensat,
    wait4::sys_wait4,
    waitid::sys_waitid,
    write::sys_write,
};

impl_syscall_nums_and_dispatch_fn! {
    SYS_GETCWD = 17              => sys_getcwd(args[..2]);
    SYS_EVENTFD2 = 19            => sys_eventfd2(args[..2]);
    SYS_EPOLL_CREATE1 = 20       => sys_epoll_create1(args[..1]);
    SYS_EPOLL_CTL = 21           => sys_epoll_ctl(args[..4]);
    SYS_EPOLL_PWAIT = 22         => sys_epoll_pwait(args[..6]);
    SYS_DUP = 23                 => sys_dup(args[..1]);
    SYS_DUP3 = 24                => sys_dup3(args[..3]);
    SYS_FCNTL = 25               => sys_fcntl(args[..3]);
    SYS_IOCTL = 29               => sys_ioctl(args[..3]);
    SYS_FLOCK = 32               => sys_flock(args[..2]);
    SYS_MKNODAT = 33             => sys_mknodat(args[..4]);
    SYS_MKDIRAT = 34             => sys_mkdirat(args[..3]);
    SYS_UNLINKAT = 35            => sys_unlinkat(args[..3]);
    SYS_SYMLINKAT = 36           => sys_symlinkat(args[..3]);
    SYS_LINKAT = 37              => sys_linkat(args[..5]);
    SYS_RENAMEAT = 38            => sys_renameat(args[..4]);
    SYS_UMOUNT = 39              => sys_umount(args[..2]);
    SYS_MOUNT = 40               => sys_mount(args[..5]);
    SYS_STATFS = 43              => sys_statfs(args[..2]);
    SYS_FSTATFS = 44             => sys_fstatfs(args[..2]);
    SYS_TRUNCATE = 45            => sys_truncate(args[..2]);
    SYS_FTRUNCATE = 46           => sys_ftruncate(args[..2]);
    SYS_FALLOCATE = 47           => sys_fallocate(args[..4]);
    SYS_FACCESSAT = 48           => sys_faccessat(args[..3]);
    SYS_CHDIR = 49               => sys_chdir(args[..1]);
    SYS_FCHDIR = 50              => sys_fchdir(args[..1]);
    SYS_CHROOT = 51              => sys_chroot(args[..1]);
    SYS_FCHMOD = 52              => sys_fchmod(args[..2]);
    SYS_FCHMODAT = 53            => sys_fchmodat(args[..3]);
    SYS_FCHOWNAT = 54            => sys_fchownat(args[..5]);
    SYS_FCHOWN = 55              => sys_fchown(args[..3]);
    SYS_OPENAT = 56              => sys_openat(args[..4]);
    SYS_CLOSE = 57               => sys_close(args[..1]);
    SYS_PIPE2 = 59               => sys_pipe2(args[..2]);
    SYS_GETDENTS64 = 61          => sys_getdents64(args[..3]);
    SYS_LSEEK = 62               => sys_lseek(args[..3]);
    SYS_READ = 63                => sys_read(args[..3]);
    SYS_WRITE = 64               => sys_write(args[..3]);
    SYS_READV = 65               => sys_readv(args[..3]);
    SYS_WRITEV = 66              => sys_writev(args[..3]);
    SYS_PREAD64 = 67             => sys_pread64(args[..4]);
    SYS_PWRITE64 = 68            => sys_pwrite64(args[..4]);
    SYS_PREADV = 69              => sys_preadv(args[..4]);
    SYS_PWRITEV = 70             => sys_pwritev(args[..4]);
    SYS_SENDFILE64 = 71          => sys_sendfile(args[..4]);
    SYS_PSELECT6 = 72            => sys_pselect6(args[..6]);
    SYS_READLINKAT = 78          => sys_readlinkat(args[..4]);
    SYS_NEWFSTATAT = 79          => sys_fstatat(args[..4]);
    SYS_NEWFSTAT = 80            => sys_fstat(args[..2]);
    SYS_SYNC = 81                => sys_sync(args[..0]);
    SYS_FSYNC = 82               => sys_fsync(args[..1]);
    SYS_FDATASYNC = 83           => sys_fdatasync(args[..1]);
    SYS_CAPGET = 90              => sys_capget(args[..2]);
    SYS_CAPSET = 91              => sys_capset(args[..2]);
    SYS_EXIT = 93                => sys_exit(args[..1]);
    SYS_EXIT_GROUP = 94          => sys_exit_group(args[..1]);
    SYS_WAITID = 95              => sys_waitid(args[..5]);
    SYS_SET_TID_ADDRESS = 96     => sys_set_tid_address(args[..1]);
    SYS_FUTEX = 98               => sys_futex(args[..6]);
    SYS_SET_ROBUST_LIST = 99     => sys_set_robust_list(args[..2]);
    SYS_NANOSLEEP = 101          => sys_nanosleep(args[..2]);
    SYS_GETITIMER = 102          => sys_getitimer(args[..2]);
    SYS_SETITIMER = 103          => sys_setitimer(args[..3]);
    SYS_TIMER_CREATE = 107       => sys_timer_create(args[..3]);
    SYS_TIMER_DELETE = 111       => sys_timer_delete(args[..1]);
    SYS_SCHED_SETAFFINITY = 122  => sys_sched_setaffinity(args[..3]);
    SYS_SCHED_GETAFFINITY = 123  => sys_sched_getaffinity(args[..3]);
    SYS_SCHED_YIELD = 124        => sys_sched_yield(args[..0]);
    SYS_KILL = 129               => sys_kill(args[..2]);
    SYS_TGKILL = 131             => sys_tgkill(args[..3]);
    SYS_SIGALTSTACK = 132        => sys_sigaltstack(args[..2]);
    SYS_RT_SIGSUSPEND = 133      => sys_rt_sigsuspend(args[..2]);
    SYS_RT_SIGACTION = 134       => sys_rt_sigaction(args[..4]);
    SYS_RT_SIGPROCMASK = 135     => sys_rt_sigprocmask(args[..4]);
    SYS_RT_SIGPENDING = 136      => sys_rt_sigpending(args[..2]);
    SYS_SET_PRIORITY = 140       => sys_set_priority(args[..3]);
    SYS_GET_PRIORITY = 141       => sys_get_priority(args[..2]);
    SYS_SETREGID = 143           => sys_setregid(args[..2]);
    SYS_SETGID = 144             => sys_setgid(args[..1]);
    SYS_SETREUID = 145           => sys_setreuid(args[..2]);
    SYS_SETUID = 146             => sys_setuid(args[..1]);
    SYS_SETRESUID = 147          => sys_setresuid(args[..3]);
    SYS_GETRESUID = 148          => sys_getresuid(args[..3]);
    SYS_SETRESGID = 149          => sys_setresgid(args[..3]);
    SYS_GETRESGID = 150          => sys_getresgid(args[..3]);
    SYS_SETFSUID = 151           => sys_setfsuid(args[..1]);
    SYS_SETFSGID = 152           => sys_setfsgid(args[..1]);
    SYS_SETPGID = 154            => sys_setpgid(args[..2]);
    SYS_GETPGID = 155            => sys_getpgid(args[..1]);
    SYS_GETSID = 156             => sys_getsid(args[..1]);
    SYS_SETSID = 157             => sys_setsid(args[..0]);
    SYS_GETGROUPS = 158          => sys_getgroups(args[..2]);
    SYS_SETGROUPS = 159          => sys_setgroups(args[..2]);
    SYS_NEWUNAME = 160           => sys_uname(args[..1]);
    SYS_GETRLIMIT = 163          => sys_getrlimit(args[..2]);
    SYS_SETRLIMIT = 164          => sys_setrlimit(args[..2]);
    SYS_GETRUSAGE = 165          => sys_getrusage(args[..2]);
    SYS_UMASK = 166              => sys_umask(args[..1]);
    SYS_PRCTL = 167              => sys_prctl(args[..5]);
    SYS_GETTIMEOFDAY = 169       => sys_gettimeofday(args[..1]);
    SYS_GETPID = 172             => sys_getpid(args[..0]);
    SYS_GETPPID = 173            => sys_getppid(args[..0]);
    SYS_GETUID = 174             => sys_getuid(args[..0]);
    SYS_GETEUID = 175            => sys_geteuid(args[..0]);
    SYS_GETGID = 176             => sys_getgid(args[..0]);
    SYS_GETEGID = 177            => sys_getegid(args[..0]);
    SYS_GETTID = 178             => sys_gettid(args[..0]);
    SYS_SEMGET = 190             => sys_semget(args[..3]);
    SYS_SEMCTL = 191             => sys_semctl(args[..4]);
    SYS_SEMOP = 193              => sys_semop(args[..3]);
    SYS_SOCKET = 198             => sys_socket(args[..3]);
    SYS_SOCKETPAIR = 199         => sys_socketpair(args[..4]);
    SYS_BIND = 200               => sys_bind(args[..3]);
    SYS_LISTEN = 201             => sys_listen(args[..2]);
    SYS_ACCEPT = 202             => sys_accept(args[..3]);
    SYS_CONNECT = 203            => sys_connect(args[..3]);
    SYS_GETSOCKNAME = 204        => sys_getsockname(args[..3]);
    SYS_GETPEERNAME = 205        => sys_getpeername(args[..3]);
    SYS_SENDTO = 206             => sys_sendto(args[..6]);
    SYS_RECVFROM = 207           => sys_recvfrom(args[..6]);
    SYS_SETSOCKOPT = 208         => sys_setsockopt(args[..5]);
    SYS_GETSOCKOPT = 209         => sys_getsockopt(args[..5]);
    SYS_SHUTDOWN = 210           => sys_shutdown(args[..2]);
    SYS_SENDMSG = 211            => sys_sendmsg(args[..3]);
    SYS_RECVMSG = 212            => sys_recvmsg(args[..3]);
    SYS_BRK = 214                => sys_brk(args[..1]);
    SYS_MUNMAP = 215             => sys_munmap(args[..2]);
    SYS_CLONE = 220              => sys_clone(args[..5], &user_ctx);
    SYS_EXECVE = 221             => sys_execve(args[..3], &mut user_ctx);
    SYS_MMAP = 222               => sys_mmap(args[..6]);
    SYS_MPROTECT = 226           => sys_mprotect(args[..3]);
    SYS_MSYNC = 227              => sys_msync(args[..3]);
    SYS_MADVISE = 233            => sys_madvise(args[..3]);
    SYS_ACCEPT4 = 242            => sys_accept4(args[..4]);
    SYS_WAIT4 = 260              => sys_wait4(args[..4]);
    SYS_PRLIMIT64 = 261          => sys_prlimit64(args[..4]);
    SYS_GETRANDOM = 278          => sys_getrandom(args[..3]);
    SYS_EXECVEAT = 281           => sys_execveat(args[..5], &mut user_ctx);
    SYS_PREADV2 = 286            => sys_preadv2(args[..5]);
    SYS_PWRITEV2 = 287           => sys_pwritev2(args[..5]);
    SYS_PRLIMIT64 = 302          => sys_prlimit64(args[..4]);
    SYS_CLOCK_GETTIME = 403      => sys_clock_gettime(args[..2]);
    SYS_CLOCK_NANOSLEEP = 407    => sys_clock_nanosleep(args[..4]);
    SYS_TIMER_GETTIME = 408      => sys_timer_gettime(args[..2]);
    SYS_TIMER_SETTIME = 409      => sys_timer_settime(args[..4]);
    SYS_UTIMENSAT = 412          => sys_utimensat(args[..4]);
    SYS_SEMTIMEDOP = 420         => sys_semtimedop(args[..4]);
    SYS_CLONE3 = 435             => sys_clone3(args[..2], &user_ctx);
}
