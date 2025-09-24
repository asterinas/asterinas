// SPDX-License-Identifier: MPL-2.0

use sctrace::{CliReporterBuilder, Patterns, SctraceBuilder, StraceLogStream};

#[test]
fn test_open_syscall() {
    let scml_content = r#"
        access_mode =
            O_RDONLY |
            O_WRONLY |
            O_RDWR;
        creation_flags =
            O_CLOEXEC |
            O_DIRECTORY |
            O_EXCL |
            O_NOCTTY |
            O_NOFOLLOW |
            O_TRUNC;
        status_flags =
            O_APPEND |
            O_ASYNC |
            O_DIRECT |
            O_LARGEFILE |
            O_NOATIME |
            O_NONBLOCK |
            O_SYNC;

        // Open an existing file
        open(
            path,
            flags = <access_mode> | <creation_flags> | <status_flags>,
        );
        openat(
            dirfd,
            path,
            flags = <access_mode> | <creation_flags> | <status_flags>,
        );

        // Create a new file
        open(
            path,
            flags = O_CREAT | <access_mode> | <creation_flags> | <status_flags>,
            mode
        );
        openat(
            dirfd,
            path,
            flags = O_CREAT | <access_mode> | <creation_flags> | <status_flags>,
            mode
        );

        // Status flags that are meaningful with O_PATH
        opath_valid_flags = O_CLOEXEC | O_DIRECTORY | O_NOFOLLOW;
        // All other flags are ignored with O_PATH
        opath_ignored_flags = O_CREAT | <creation_flags> | <status_flags>;
        // Obtain a file descriptor to indicate a location in FS
        open(
            path,
            flags = O_PATH | <opath_valid_flags> | <opath_ignored_flags>
        );
        openat(
            dirfd,
            path,
            flags = O_PATH | <opath_valid_flags> | <opath_ignored_flags>
        );

        // Create an unnamed file
        // open(path, flags = O_TMPFILE | <creation_flags> | <status_flags>)
    "#;

    let log_lines = r#"
        openat(AT_FDCWD, "/lib/aarch64-linux-gnu/libc.so.6", O_RDONLY|O_CLOEXEC) = 3
        open("/dev/tdx_guest", O_RDWR|O_NONBLOCK) = 3
        open("/tmp/sctrace_testfile", O_CREAT|O_RDWR|O_CLOEXEC, 0666) = 4
        openat(AT_FDCWD, "/tmp/sctrace_testfile2", O_PATH|O_CREAT) = 5
    "#;

    let sctrace = SctraceBuilder::new()
        .patterns(Patterns::from_scml(scml_content).unwrap())
        .strace(StraceLogStream::from_string(log_lines).unwrap())
        .reporter(CliReporterBuilder::new().quiet().collect().build())
        .build();

    let result = sctrace.run().unwrap().unwrap();
    assert_eq!(result.len(), 0);
}

#[test]
fn test_timer_create_syscall() {
    let scml_content = r#"
        opt_notify_methods = SIGEV_NONE | SIGEV_SIGNAL | SIGEV_THREAD_ID;

        // Create a timer with predefined clock source
        timer_create(
            clockid = CLOCK_PROCESS_CPUTIME_ID | CLOCK_THREAD_CPUTIME_ID | CLOCK_REALTIME | CLOCK_MONOTONIC | CLOCK_BOOTTIME,
            sevp = {
                sigev_notify = <opt_notify_methods>,
                ..
            },
            timerid
        );

        // Create a timer based on a per-process or per-thread clock
        timer_create(
            clockid = <INTEGER>,
            sevp = {
                sigev_notify = <opt_notify_methods>,
                ..
            },
            timerid
        );
    "#;

    let log_lines = r#"
        timer_create(CLOCK_REALTIME, {sigev_notify=SIGEV_SIGNAL, sigev_signo=SIGALRM, sigev_value={sival_ptr=0x559b4d3e2e70}}, 0x7ffcb1f4d9c0) = 0
        timer_create(0xff5be79e /* CLOCK_??? */, {sigev_value={sival_int=565425088, sival_ptr=0x562221b3b3c0}, sigev_signo=SIGRTMIN, sigev_notify=SIGEV_THREAD_ID, sigev_notify_thread_id=1344269}, [0]) = 0
    "#;

    let sctrace = SctraceBuilder::new()
        .patterns(Patterns::from_scml(scml_content).unwrap())
        .strace(StraceLogStream::from_string(log_lines).unwrap())
        .reporter(CliReporterBuilder::new().quiet().collect().build())
        .build();

    let result = sctrace.run().unwrap().unwrap();
    assert_eq!(result.len(), 0);
}

#[test]
fn test_multiple_struct_with_same_name() {
    let scml_content = r#"
        struct cmsghdr = {
            cmsg_level = SOL_SOCKET,
            cmsg_type  = SO_TIMESTAMP_OLD | SCM_RIGHTS | SCM_CREDENTIALS,
            ..
        };
        struct cmsghdr = {
            cmsg_level = SOL_IP,
            cmsg_type  = IP_TTL,
            ..
        };

        // Rule for message header, which refers to the rules for control message header
        struct msghdr = {
            msg_control = [ <cmsghdr> ],
            ..
        };

        recvmsg(socket, message = <msghdr>, flags);
    "#;

    let log_lines = &[
        "recvmsg(4, {msg_name=NULL, msg_namelen=0, msg_iov=NULL, msg_iovlen=0, msg_control=[{cmsg_len=16, cmsg_level=SOL_SOCKET, cmsg_type=SCM_RIGHTS}], msg_controllen=16, msg_flags=0}, 0) = 24",
        "recvmsg(5, {msg_name=NULL, msg_namelen=0, msg_iov=NULL, msg_iovlen=0, msg_control=[{cmsg_len=16, cmsg_level=SOL_IP, cmsg_type=IP_TTL}], msg_controllen=16, msg_flags=0}, 0) = 24",
        "recvmsg(6, {msg_name=NULL, msg_namelen=0, msg_iov=NULL, msg_iovlen=0, msg_control=[{cmsg_len=16, cmsg_level=SOL_IPV6, cmsg_type=IPV6_UNICAST_HOPS}], msg_controllen=16, msg_flags=0}, 0) = 24",
    ];

    let sctrace = SctraceBuilder::new()
        .patterns(Patterns::from_scml(scml_content).unwrap())
        .strace(StraceLogStream::from_string(log_lines.join("\n").as_str()).unwrap())
        .reporter(CliReporterBuilder::new().quiet().collect().build())
        .build();

    let result = sctrace.run().unwrap().unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], format!("Unsupported syscall: {}", log_lines[2]));
}

#[test]
fn test_clone_syscall() {
    let scml_content = r#"
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
    "#;

    let log_lines = r#"
        clone(child_stack=NULL, flags=CLONE_CHILD_CLEARTID|CLONE_CHILD_SETTID|SIGCHLD, child_tidptr=0x7f7745c1ca10) = 141614
    "#;

    let sctrace = SctraceBuilder::new()
        .patterns(Patterns::from_scml(scml_content).unwrap())
        .strace(StraceLogStream::from_string(log_lines).unwrap())
        .reporter(CliReporterBuilder::new().quiet().collect().build())
        .build();

    let result = sctrace.run().unwrap().unwrap();
    assert_eq!(result.len(), 0);
}

#[test]
fn test_multiple_threads_syscalls() {
    let scml_content = r#"
        wait4(
            pid, wstatus,
            options = WNOHANG | WSTOPPED | WCONTINUED | WNOWAIT,
            rusage
        );
    "#;

    let log_lines = r#"
        141611 wait4(-1,  <unfinished ...>
        141611 <... wait4 resumed>[{WIFEXITED(s) && WEXITSTATUS(s) == 0}], WNOHANG, NULL) = 141612
    "#;

    let sctrace = SctraceBuilder::new()
        .patterns(Patterns::from_scml(scml_content).unwrap())
        .strace(StraceLogStream::from_string(log_lines).unwrap())
        .reporter(CliReporterBuilder::new().quiet().collect().build())
        .build();

    let result = sctrace.run().unwrap().unwrap();
    assert_eq!(result.len(), 0);
}

#[test]
fn test_check_logfile_wildcard_pattern() {
    let scml_content = r#"
        openat(dirfd, pathname, flags, ..);
    "#;

    let log_lines = r#"
        openat(AT_FDCWD, "/etc/ld.so.cache", O_RDONLY|O_CLOEXEC) = 3
        openat(AT_FDCWD, "/lib/x86_64-linux-gnu/libc.so.6", O_RDONLY|O_CLOEXEC, 0755) = 4
    "#;

    let sctrace = SctraceBuilder::new()
        .patterns(Patterns::from_scml(scml_content).unwrap())
        .strace(StraceLogStream::from_string(log_lines).unwrap())
        .reporter(CliReporterBuilder::new().quiet().collect().build())
        .build();

    let result = sctrace.run().unwrap().unwrap();
    assert_eq!(result.len(), 0);
}

#[test]
fn test_check_program_simple_command() {
    let scml_content = r#"
        execve(filename, argv, envp);
    "#;

    let sctrace = SctraceBuilder::new()
        .patterns(Patterns::from_scml(scml_content).unwrap())
        .strace(StraceLogStream::run_cmd("/bin/true", vec![]).unwrap())
        .reporter(CliReporterBuilder::new().quiet().collect().build())
        .build();

    let result = sctrace.run().unwrap().unwrap();
    assert!(result.iter().all(|error| !error.contains("execve(")));
}

#[test]
fn test_heterogeneous_arrays() {
    let scml_content = r#"
        struct iovec = {
            iov_base = [
                [
                    {
                        nlmsg_type = RTM_NEWADDR,
                        ..
                    },
                    [
                        [ { nla_type = IFA_CACHEINFO, .. } ]
                    ]
                ]
            ],
            ..
        };
        recvmsg(
            sockfd,
            msg = {
                msg_iov = [ <iovec> ],
                ..
            },
            flags
        );
    "#;

    let log_lines = r#"
        1370921 recvmsg(7<socket:[181857552]>, {msg_name={sa_family=AF_NETLINK, nl_pid=0, nl_groups=00000000}, msg_namelen=0, msg_iov=[{iov_base=[[{nlmsg_len=76, nlmsg_type=RTM_NEWADDR, nlmsg_flags=NLM_F_MULTI, nlmsg_seq=1758615457, nlmsg_pid=1370920}, [[{nla_len=20, nla_type=IFA_CACHEINFO}]]]], iov_len=4096}], msg_iovlen=1, msg_control=NULL, msg_controllen=0, msg_flags=0}, 0) = 1280
    "#;

    let sctrace = SctraceBuilder::new()
        .patterns(Patterns::from_scml(scml_content).unwrap())
        .strace(StraceLogStream::from_string(log_lines).unwrap())
        .reporter(CliReporterBuilder::new().quiet().collect().build())
        .build();

    let result = sctrace.run().unwrap().unwrap();
    assert_eq!(result.len(), 0);
}
