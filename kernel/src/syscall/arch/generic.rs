// SPDX-License-Identifier: MPL-2.0

/// Imports all generic syscall entries.
///
/// This macro is intended solely for internal use within [macro@define_syscalls_with_generic_syscall_table].
///
/// The primary reason for this macro's existence is to facilitate code formatting.
/// The macro, [macro@define_syscalls_with_generic_syscall_table],
/// contains non-standard syntax that rustfmt cannot process.
/// By moving these use statements into this dedicated macro, they can be correctly formatted.
macro_rules! import_generic_syscall_entries {
    () => {
        use $crate::syscall::{
            accept::{sys_accept, sys_accept4},
            access::{sys_faccessat, sys_faccessat2},
            bind::sys_bind,
            brk::sys_brk,
            capget::sys_capget,
            capset::sys_capset,
            chdir::{sys_chdir, sys_fchdir},
            chmod::{sys_fchmod, sys_fchmodat, sys_fchmodat2},
            chown::{sys_fchown, sys_fchownat},
            chroot::sys_chroot,
            clock_gettime::sys_clock_gettime,
            clone::{sys_clone, sys_clone3},
            close::{sys_close, sys_close_range},
            connect::sys_connect,
            dup::{sys_dup, sys_dup3},
            epoll::{sys_epoll_create1, sys_epoll_ctl, sys_epoll_pwait, sys_epoll_pwait2},
            eventfd::sys_eventfd2,
            execve::{sys_execve, sys_execveat},
            exit::sys_exit,
            exit_group::sys_exit_group,
            fadvise64::sys_fadvise64,
            fallocate::sys_fallocate,
            fcntl::sys_fcntl,
            flock::sys_flock,
            fsync::{sys_fdatasync, sys_fsync},
            futex::sys_futex,
            get_ioprio::sys_ioprio_get,
            get_priority::sys_get_priority,
            getcpu::sys_getcpu,
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
            getxattr::{sys_fgetxattr, sys_getxattr, sys_lgetxattr},
            inotify::{sys_inotify_add_watch, sys_inotify_init1, sys_inotify_rm_watch},
            ioctl::sys_ioctl,
            kill::sys_kill,
            link::sys_linkat,
            listen::sys_listen,
            listxattr::{sys_flistxattr, sys_listxattr, sys_llistxattr},
            lseek::sys_lseek,
            madvise::sys_madvise,
            memfd_create::sys_memfd_create,
            mkdir::sys_mkdirat,
            mknod::sys_mknodat,
            mmap::sys_mmap,
            mount::sys_mount,
            mprotect::sys_mprotect,
            mremap::sys_mremap,
            msync::sys_msync,
            munmap::sys_munmap,
            nanosleep::{sys_clock_nanosleep, sys_nanosleep},
            open::sys_openat,
            pidfd_getfd::sys_pidfd_getfd,
            pidfd_open::sys_pidfd_open,
            pipe::sys_pipe2,
            ppoll::sys_ppoll,
            prctl::sys_prctl,
            pread64::sys_pread64,
            preadv::{sys_preadv, sys_preadv2, sys_readv},
            prlimit64::{sys_getrlimit, sys_prlimit64, sys_setrlimit},
            pselect6::sys_pselect6,
            pwrite64::sys_pwrite64,
            pwritev::{sys_pwritev, sys_pwritev2, sys_writev},
            read::sys_read,
            readlink::sys_readlinkat,
            reboot::sys_reboot,
            recvfrom::sys_recvfrom,
            recvmsg::sys_recvmsg,
            removexattr::{sys_fremovexattr, sys_lremovexattr, sys_removexattr},
            rename::sys_renameat2,
            rt_sigaction::sys_rt_sigaction,
            rt_sigpending::sys_rt_sigpending,
            rt_sigprocmask::sys_rt_sigprocmask,
            rt_sigreturn::sys_rt_sigreturn,
            rt_sigsuspend::sys_rt_sigsuspend,
            rt_sigtimedwait::sys_rt_sigtimedwait,
            sched_affinity::{sys_sched_getaffinity, sys_sched_setaffinity},
            sched_get_priority_max::sys_sched_get_priority_max,
            sched_get_priority_min::sys_sched_get_priority_min,
            sched_getattr::sys_sched_getattr,
            sched_getparam::sys_sched_getparam,
            sched_getscheduler::sys_sched_getscheduler,
            sched_setattr::sys_sched_setattr,
            sched_setparam::sys_sched_setparam,
            sched_setscheduler::sys_sched_setscheduler,
            sched_yield::sys_sched_yield,
            semctl::sys_semctl,
            semget::sys_semget,
            semop::{sys_semop, sys_semtimedop},
            sendfile::sys_sendfile,
            sendmmsg::sys_sendmmsg,
            sendmsg::sys_sendmsg,
            sendto::sys_sendto,
            set_ioprio::sys_ioprio_set,
            set_priority::sys_set_priority,
            set_robust_list::sys_set_robust_list,
            set_tid_address::sys_set_tid_address,
            setdomainname::sys_setdomainname,
            setfsgid::sys_setfsgid,
            setfsuid::sys_setfsuid,
            setgid::sys_setgid,
            setgroups::sys_setgroups,
            sethostname::sys_sethostname,
            setitimer::{sys_getitimer, sys_setitimer},
            setns::sys_setns,
            setpgid::sys_setpgid,
            setregid::sys_setregid,
            setresgid::sys_setresgid,
            setresuid::sys_setresuid,
            setreuid::sys_setreuid,
            setsid::sys_setsid,
            setsockopt::sys_setsockopt,
            setuid::sys_setuid,
            setxattr::{sys_fsetxattr, sys_lsetxattr, sys_setxattr},
            shutdown::sys_shutdown,
            sigaltstack::sys_sigaltstack,
            signalfd::sys_signalfd4,
            socket::sys_socket,
            socketpair::sys_socketpair,
            stat::{sys_fstat, sys_fstatat},
            statfs::{sys_fstatfs, sys_statfs},
            statx::sys_statx,
            symlink::sys_symlinkat,
            sync::{sys_sync, sys_syncfs},
            sysinfo::sys_sysinfo,
            tgkill::sys_tgkill,
            timer_create::{sys_timer_create, sys_timer_delete},
            timer_settime::{sys_timer_gettime, sys_timer_settime},
            timerfd_create::sys_timerfd_create,
            timerfd_gettime::sys_timerfd_gettime,
            timerfd_settime::sys_timerfd_settime,
            truncate::{sys_ftruncate, sys_truncate},
            umask::sys_umask,
            umount::sys_umount,
            uname::sys_uname,
            unlink::sys_unlinkat,
            unshare::sys_unshare,
            utimens::sys_utimensat,
            wait4::sys_wait4,
            waitid::sys_waitid,
            write::sys_write,
        };
    };
}

pub(super) use import_generic_syscall_entries;

/// Defines syscalls by combining architecture-specific definitions with the generic syscall table.
///
/// Generic syscalls refer to syscalls whose numbers are standardized across multiple architectures.
/// By using this macro, you only need to define the arch-specific syscalls;
/// the generic ones are integrated automatically.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/asm-generic/unistd.h>.
///
/// # Examples
///
/// Define architecture-specific syscalls `example1` and `example2` with generic syscalls:
///
/// ```rust
/// define_syscalls_with_generic_syscall_table! {
///     SYS_EXAMPLE1 = 500 => sys_example1(args[..0]);
///     SYS_EXAMPLE2 = 501 => sys_example2(args[..1], &user_ctx);
/// }
/// ```
macro_rules! define_syscalls_with_generic_syscall_table {
    ( $( $name: ident = $num: literal => $handler: ident $args: tt );* $(;)? ) => {
        generic::import_generic_syscall_entries!();

        $crate::syscall::impl_syscall_nums_and_dispatch_fn! {
            // Generic syscalls
            SYS_SETXATTR = 5                 => sys_setxattr(args[..5]);
            SYS_LSETXATTR = 6                => sys_lsetxattr(args[..5]);
            SYS_FSETXATTR = 7                => sys_fsetxattr(args[..5]);
            SYS_GETXATTR = 8                 => sys_getxattr(args[..4]);
            SYS_LGETXATTR = 9                => sys_lgetxattr(args[..4]);
            SYS_FGETXATTR = 10               => sys_fgetxattr(args[..4]);
            SYS_LISTXATTR = 11               => sys_listxattr(args[..3]);
            SYS_LLISTXATTR = 12              => sys_llistxattr(args[..3]);
            SYS_FLISTXATTR = 13              => sys_flistxattr(args[..3]);
            SYS_REMOVEXATTR = 14             => sys_removexattr(args[..2]);
            SYS_LREMOVEXATTR = 15            => sys_lremovexattr(args[..2]);
            SYS_FREMOVEXATTR = 16            => sys_fremovexattr(args[..2]);
            SYS_GETCWD = 17                  => sys_getcwd(args[..2]);
            SYS_EVENTFD2 = 19                => sys_eventfd2(args[..2]);
            SYS_EPOLL_CREATE1 = 20           => sys_epoll_create1(args[..1]);
            SYS_EPOLL_CTL = 21               => sys_epoll_ctl(args[..4]);
            SYS_EPOLL_PWAIT = 22             => sys_epoll_pwait(args[..6]);
            SYS_DUP = 23                     => sys_dup(args[..1]);
            SYS_DUP3 = 24                    => sys_dup3(args[..3]);
            SYS_FCNTL = 25                   => sys_fcntl(args[..3]);
            SYS_INOTIFY_INIT1 = 26           => sys_inotify_init1(args[..1]);
            SYS_INOTIFY_ADD_WATCH = 27       => sys_inotify_add_watch(args[..3]);
            SYS_INOTIFY_RM_WATCH = 28        => sys_inotify_rm_watch(args[..2]);
            SYS_IOCTL = 29                   => sys_ioctl(args[..3]);
            SYS_IOPRIO_SET = 30              => sys_ioprio_set(args[..3]);
            SYS_IOPRIO_GET = 31              => sys_ioprio_get(args[..2]);
            SYS_FLOCK = 32                   => sys_flock(args[..2]);
            SYS_MKNODAT = 33                 => sys_mknodat(args[..4]);
            SYS_MKDIRAT = 34                 => sys_mkdirat(args[..3]);
            SYS_UNLINKAT = 35                => sys_unlinkat(args[..3]);
            SYS_SYMLINKAT = 36               => sys_symlinkat(args[..3]);
            SYS_LINKAT = 37                  => sys_linkat(args[..5]);
            SYS_UMOUNT = 39                  => sys_umount(args[..2]);
            SYS_MOUNT = 40                   => sys_mount(args[..5]);
            SYS_STATFS = 43                  => sys_statfs(args[..2]);
            SYS_FSTATFS = 44                 => sys_fstatfs(args[..2]);
            SYS_TRUNCATE = 45                => sys_truncate(args[..2]);
            SYS_FTRUNCATE = 46               => sys_ftruncate(args[..2]);
            SYS_FALLOCATE = 47               => sys_fallocate(args[..4]);
            SYS_FACCESSAT = 48               => sys_faccessat(args[..3]);
            SYS_CHDIR = 49                   => sys_chdir(args[..1]);
            SYS_FCHDIR = 50                  => sys_fchdir(args[..1]);
            SYS_CHROOT = 51                  => sys_chroot(args[..1]);
            SYS_FCHMOD = 52                  => sys_fchmod(args[..2]);
            SYS_FCHMODAT = 53                => sys_fchmodat(args[..3]);
            SYS_FCHOWNAT = 54                => sys_fchownat(args[..5]);
            SYS_FCHOWN = 55                  => sys_fchown(args[..3]);
            SYS_OPENAT = 56                  => sys_openat(args[..4]);
            SYS_CLOSE = 57                   => sys_close(args[..1]);
            SYS_PIPE2 = 59                   => sys_pipe2(args[..2]);
            SYS_GETDENTS64 = 61              => sys_getdents64(args[..3]);
            SYS_LSEEK = 62                   => sys_lseek(args[..3]);
            SYS_READ = 63                    => sys_read(args[..3]);
            SYS_WRITE = 64                   => sys_write(args[..3]);
            SYS_READV = 65                   => sys_readv(args[..3]);
            SYS_WRITEV = 66                  => sys_writev(args[..3]);
            SYS_PREAD64 = 67                 => sys_pread64(args[..4]);
            SYS_PWRITE64 = 68                => sys_pwrite64(args[..4]);
            SYS_PREADV = 69                  => sys_preadv(args[..5]);
            SYS_PWRITEV = 70                 => sys_pwritev(args[..5]);
            SYS_SENDFILE64 = 71              => sys_sendfile(args[..4]);
            SYS_PSELECT6 = 72                => sys_pselect6(args[..6]);
            SYS_PPOLL = 73                   => sys_ppoll(args[..5]);
            SYS_SIGNALFD4 = 74               => sys_signalfd4(args[..4]);
            SYS_READLINKAT = 78              => sys_readlinkat(args[..4]);
            SYS_NEWFSTATAT = 79              => sys_fstatat(args[..4]);
            SYS_NEWFSTAT = 80                => sys_fstat(args[..2]);
            SYS_SYNC = 81                    => sys_sync(args[..0]);
            SYS_FSYNC = 82                   => sys_fsync(args[..1]);
            SYS_FDATASYNC = 83               => sys_fdatasync(args[..1]);
            SYS_TIMERFD_CREATE = 85          => sys_timerfd_create(args[..2]);
            SYS_TIMERFD_SETTIME = 86         => sys_timerfd_settime(args[..4]);
            SYS_TIMERFD_GETTIME = 87         => sys_timerfd_gettime(args[..2]);
            SYS_UTIMENSAT = 88               => sys_utimensat(args[..4]);
            SYS_CAPGET = 90                  => sys_capget(args[..2]);
            SYS_CAPSET = 91                  => sys_capset(args[..2]);
            SYS_EXIT = 93                    => sys_exit(args[..1]);
            SYS_EXIT_GROUP = 94              => sys_exit_group(args[..1]);
            SYS_WAITID = 95                  => sys_waitid(args[..5]);
            SYS_SET_TID_ADDRESS = 96         => sys_set_tid_address(args[..1]);
            SYS_UNSHARE = 97                 => sys_unshare(args[..1]);
            SYS_FUTEX = 98                   => sys_futex(args[..6]);
            SYS_SET_ROBUST_LIST = 99         => sys_set_robust_list(args[..2]);
            SYS_NANOSLEEP = 101              => sys_nanosleep(args[..2]);
            SYS_GETITIMER = 102              => sys_getitimer(args[..2]);
            SYS_SETITIMER = 103              => sys_setitimer(args[..3]);
            SYS_TIMER_CREATE = 107           => sys_timer_create(args[..3]);
            SYS_TIMER_GETTIME = 108          => sys_timer_gettime(args[..2]);
            SYS_TIMER_SETTIME = 110          => sys_timer_settime(args[..4]);
            SYS_TIMER_DELETE = 111           => sys_timer_delete(args[..1]);
            SYS_CLOCK_GETTIME = 113          => sys_clock_gettime(args[..2]);
            SYS_CLOCK_NANOSLEEP = 115        => sys_clock_nanosleep(args[..4]);
            SYS_SCHED_SETPARAM = 118         => sys_sched_setparam(args[..2]);
            SYS_SCHED_SETSCHEDULER = 119     => sys_sched_setscheduler(args[..3]);
            SYS_SCHED_GETSCHEDULER = 120     => sys_sched_getscheduler(args[..1]);
            SYS_SCHED_GETPARAM = 121         => sys_sched_getparam(args[..2]);
            SYS_SCHED_SETAFFINITY = 122      => sys_sched_setaffinity(args[..3]);
            SYS_SCHED_GETAFFINITY = 123      => sys_sched_getaffinity(args[..3]);
            SYS_SCHED_YIELD = 124            => sys_sched_yield(args[..0]);
            SYS_SCHED_GET_PRIORITY_MAX = 125 => sys_sched_get_priority_max(args[..1]);
            SYS_SCHED_GET_PRIORITY_MIN = 126 => sys_sched_get_priority_min(args[..1]);
            SYS_KILL = 129                   => sys_kill(args[..2]);
            SYS_TGKILL = 131                 => sys_tgkill(args[..3]);
            SYS_SIGALTSTACK = 132            => sys_sigaltstack(args[..2], &user_ctx);
            SYS_RT_SIGSUSPEND = 133          => sys_rt_sigsuspend(args[..2]);
            SYS_RT_SIGACTION = 134           => sys_rt_sigaction(args[..4]);
            SYS_RT_SIGPROCMASK = 135         => sys_rt_sigprocmask(args[..4]);
            SYS_RT_SIGPENDING = 136          => sys_rt_sigpending(args[..2]);
            SYS_RT_SIGTIMEDWAIT = 137        => sys_rt_sigtimedwait(args[..4]);
            SYS_RT_SIGRETURN = 139           => sys_rt_sigreturn(args[..0], &mut user_ctx);
            SYS_SET_PRIORITY = 140           => sys_set_priority(args[..3]);
            SYS_GET_PRIORITY = 141           => sys_get_priority(args[..2]);
            SYS_REBOOT = 142                 => sys_reboot(args[..4]);
            SYS_SETREGID = 143               => sys_setregid(args[..2]);
            SYS_SETGID = 144                 => sys_setgid(args[..1]);
            SYS_SETREUID = 145               => sys_setreuid(args[..2]);
            SYS_SETUID = 146                 => sys_setuid(args[..1]);
            SYS_SETRESUID = 147              => sys_setresuid(args[..3]);
            SYS_GETRESUID = 148              => sys_getresuid(args[..3]);
            SYS_SETRESGID = 149              => sys_setresgid(args[..3]);
            SYS_GETRESGID = 150              => sys_getresgid(args[..3]);
            SYS_SETFSUID = 151               => sys_setfsuid(args[..1]);
            SYS_SETFSGID = 152               => sys_setfsgid(args[..1]);
            SYS_SETPGID = 154                => sys_setpgid(args[..2]);
            SYS_GETPGID = 155                => sys_getpgid(args[..1]);
            SYS_GETSID = 156                 => sys_getsid(args[..1]);
            SYS_SETSID = 157                 => sys_setsid(args[..0]);
            SYS_GETGROUPS = 158              => sys_getgroups(args[..2]);
            SYS_SETGROUPS = 159              => sys_setgroups(args[..2]);
            SYS_NEWUNAME = 160               => sys_uname(args[..1]);
            SYS_SETHOSTNAME = 161            => sys_sethostname(args[..2]);
            SYS_SETDOMAINNAME = 162          => sys_setdomainname(args[..2]);
            SYS_GETRLIMIT = 163              => sys_getrlimit(args[..2]);
            SYS_SETRLIMIT = 164              => sys_setrlimit(args[..2]);
            SYS_GETRUSAGE = 165              => sys_getrusage(args[..2]);
            SYS_UMASK = 166                  => sys_umask(args[..1]);
            SYS_PRCTL = 167                  => sys_prctl(args[..5]);
            SYS_GETCPU = 168                 => sys_getcpu(args[..3]);
            SYS_GETTIMEOFDAY = 169           => sys_gettimeofday(args[..1]);
            SYS_GETPID = 172                 => sys_getpid(args[..0]);
            SYS_GETPPID = 173                => sys_getppid(args[..0]);
            SYS_GETUID = 174                 => sys_getuid(args[..0]);
            SYS_GETEUID = 175                => sys_geteuid(args[..0]);
            SYS_GETGID = 176                 => sys_getgid(args[..0]);
            SYS_GETEGID = 177                => sys_getegid(args[..0]);
            SYS_GETTID = 178                 => sys_gettid(args[..0]);
            SYS_SYSINFO = 179                => sys_sysinfo(args[..1]);
            SYS_SEMGET = 190                 => sys_semget(args[..3]);
            SYS_SEMCTL = 191                 => sys_semctl(args[..4]);
            SYS_SEMTIMEDOP = 192             => sys_semtimedop(args[..4]);
            SYS_SEMOP = 193                  => sys_semop(args[..3]);
            SYS_SOCKET = 198                 => sys_socket(args[..3]);
            SYS_SOCKETPAIR = 199             => sys_socketpair(args[..4]);
            SYS_BIND = 200                   => sys_bind(args[..3]);
            SYS_LISTEN = 201                 => sys_listen(args[..2]);
            SYS_ACCEPT = 202                 => sys_accept(args[..3]);
            SYS_CONNECT = 203                => sys_connect(args[..3]);
            SYS_GETSOCKNAME = 204            => sys_getsockname(args[..3]);
            SYS_GETPEERNAME = 205            => sys_getpeername(args[..3]);
            SYS_SENDTO = 206                 => sys_sendto(args[..6]);
            SYS_RECVFROM = 207               => sys_recvfrom(args[..6]);
            SYS_SETSOCKOPT = 208             => sys_setsockopt(args[..5]);
            SYS_GETSOCKOPT = 209             => sys_getsockopt(args[..5]);
            SYS_SHUTDOWN = 210               => sys_shutdown(args[..2]);
            SYS_SENDMSG = 211                => sys_sendmsg(args[..3]);
            SYS_RECVMSG = 212                => sys_recvmsg(args[..3]);
            SYS_BRK = 214                    => sys_brk(args[..1]);
            SYS_MUNMAP = 215                 => sys_munmap(args[..2]);
            SYS_MREMAP = 216                 => sys_mremap(args[..5]);
            SYS_CLONE = 220                  => sys_clone(args[..5], &user_ctx);
            SYS_EXECVE = 221                 => sys_execve(args[..3], &mut user_ctx);
            SYS_MMAP = 222                   => sys_mmap(args[..6]);
            SYS_FADVISE64 = 223              => sys_fadvise64(args[..4]);
            SYS_MPROTECT = 226               => sys_mprotect(args[..3]);
            SYS_MSYNC = 227                  => sys_msync(args[..3]);
            SYS_MADVISE = 233                => sys_madvise(args[..3]);
            SYS_ACCEPT4 = 242                => sys_accept4(args[..4]);
            SYS_WAIT4 = 260                  => sys_wait4(args[..4]);
            SYS_PRLIMIT64 = 261              => sys_prlimit64(args[..4]);
            SYS_SYNCFS = 267                 => sys_syncfs(args[..1]);
            SYS_SETNS = 268                  => sys_setns(args[..2]);
            SYS_SENDMMSG = 269               => sys_sendmmsg(args[..4]);
            SYS_SCHED_SETATTR = 274          => sys_sched_setattr(args[..3]);
            SYS_SCHED_GETATTR = 275          => sys_sched_getattr(args[..4]);
            SYS_RENAMEAT2 = 276              => sys_renameat2(args[..5]);
            SYS_GETRANDOM = 278              => sys_getrandom(args[..3]);
            SYS_MEMFD_CREATE = 279           => sys_memfd_create(args[..2]);
            SYS_EXECVEAT = 281               => sys_execveat(args[..5], &mut user_ctx);
            SYS_PREADV2 = 286                => sys_preadv2(args[..6]);
            SYS_PWRITEV2 = 287               => sys_pwritev2(args[..6]);
            SYS_STATX = 291                  => sys_statx(args[..5]);
            SYS_PIDFD_OPEN = 434             => sys_pidfd_open(args[..2]);
            SYS_CLONE3 = 435                 => sys_clone3(args[..2], &user_ctx);
            SYS_CLOSE_RANGE = 436            => sys_close_range(args[..3]);
            SYS_PIDFD_GETFD = 438            => sys_pidfd_getfd(args[..3]);
            SYS_FACCESSAT2 = 439             => sys_faccessat2(args[..4]);
            SYS_EPOLL_PWAIT2 = 441           => sys_epoll_pwait2(args[..5]);
            SYS_FCHMODAT2 = 452              => sys_fchmodat2(args[..4]);
            // Architecture-specific syscalls
            $( $name = $num => $handler $args );*
        }
    };
}

pub(super) use define_syscalls_with_generic_syscall_table;
