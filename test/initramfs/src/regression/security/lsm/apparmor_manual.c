// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ptrace.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#define ENFORCE_PROFILE "apparmor/m1-capability-test"
#define COMPLAIN_PROFILE "apparmor/m1-complain-test"
#define DEFAULT_PROFILE "kernel/default"
#define ENFORCE_DENY_PATH "/test/security/lsm/apparmor-enforce-deny"

static const char *entry_name(const char *path)
{
    const char *slash = strrchr(path, '/');
    return slash == NULL ? path : slash + 1;
}

static int report(const char *step, const char *executable,
                  const char *expected_profile, const char *mode,
                  const char *operation, int ret, int error, int expected_ret,
                  int expected_error, const char *expected)
{
    int pass = ret == expected_ret && error == expected_error;

    printf("STEP=%s\n", step);
    printf("EXECUTABLE=%s\n", executable);
    printf("EXPECTED_PROFILE=%s\n", expected_profile);
    printf("PROFILE_QUERY=UNAVAILABLE\n");
    printf("MODE=%s\n", mode);
    printf("OPERATION=%s\n", operation);
    if (strcmp(operation, "chroot") == 0 ||
        strcmp(operation, "fork->chroot") == 0 ||
        strcmp(operation, "ptrace(PTRACE_ATTACH)") == 0) {
        printf("AUDIT_CHECK=MANUAL_REQUIRED\n");
    }
    if (strcmp(mode, "complain") == 0) {
        printf("AUDIT_EXPECTED=complain/would-deny\n");
    }
    printf("RETURN=%d\n", ret);
    printf("ERRNO=%d\n", error);
    printf("EXPECTED=%s\n", expected);
    printf("RESULT_SCOPE=USERSPACE_SYSCALL\n");
    printf("RESULT=%s\n", pass ? "PASS" : "FAIL");
    return pass ? 0 : 1;
}

static int report_setup_failure(const char *step, const char *executable,
                                const char *expected_profile,
                                const char *mode, const char *operation,
                                int error, const char *expected)
{
    printf("STEP=%s\n", step);
    printf("EXECUTABLE=%s\n", executable);
    printf("EXPECTED_PROFILE=%s\n", expected_profile);
    printf("PROFILE_QUERY=UNAVAILABLE\n");
    printf("MODE=%s\n", mode);
    printf("OPERATION=%s\n", operation);
    if (strcmp(operation, "chroot") == 0 ||
        strcmp(operation, "fork->chroot") == 0 ||
        strcmp(operation, "ptrace(PTRACE_ATTACH)") == 0) {
        printf("AUDIT_CHECK=MANUAL_REQUIRED\n");
    }
    if (strcmp(mode, "complain") == 0) {
        printf("AUDIT_EXPECTED=complain/would-deny\n");
    }
    printf("RETURN=NOT_EXECUTED\n");
    printf("ERRNO=%d\n", error);
    printf("EXPECTED=%s\n", expected);
    printf("RESULT_SCOPE=USERSPACE_SYSCALL\n");
    printf("RESULT=FAIL\n");
    return 1;
}

static int run_chroot(const char *step, const char *executable,
                      const char *profile, const char *mode, int expected_ret,
                      int expected_errno, const char *expected)
{
    char dir[] = "/tmp/apparmor-manual-chroot-XXXXXX";
    int ret;
    int error;

    if (mkdtemp(dir) == NULL) {
        return report_setup_failure(step, executable, profile, mode, "chroot",
                                    errno, expected);
    }

    errno = 0;
    ret = chroot(dir);
    error = errno;
    if (ret == -1) {
        rmdir(dir);
    }
    return report(step, executable, profile, mode, "chroot", ret, error,
                  expected_ret, expected_errno, expected);
}

static int run_fchown(const char *executable)
{
    char file_path[] = "/tmp/apparmor-manual-chown-XXXXXX";
    int fd = mkstemp(file_path);
    int ret;
    int error;

    if (fd == -1) {
        return report_setup_failure("enforce-selective-allow", executable,
                                    ENFORCE_PROFILE, "enforce", "fchown",
                                    errno, "RETURN=0,ERRNO=0");
    }

    errno = 0;
    ret = fchown(fd, 1, 1);
    error = errno;
    close(fd);
    unlink(file_path);
    return report("enforce-selective-allow", executable, ENFORCE_PROFILE,
                  "enforce", "fchown", ret, error, 0, 0,
                  "RETURN=0,ERRNO=0");
}

struct child_result {
    int ret;
    int error;
};

static int run_fork(const char *executable)
{
    char dir[] = "/tmp/apparmor-manual-fork-XXXXXX";
    struct child_result child_result = { -1, 0 };
    int pipe_fds[2];
    pid_t child_pid;
    int status;

    if (mkdtemp(dir) == NULL) {
        return report_setup_failure("fork-inheritance", executable,
                                    ENFORCE_PROFILE, "enforce",
                                    "fork->chroot", errno,
                                    "RETURN=-1,ERRNO=1(EPERM)");
    }
    if (pipe(pipe_fds) == -1) {
        int error = errno;
        rmdir(dir);
        return report_setup_failure("fork-inheritance", executable,
                                    ENFORCE_PROFILE, "enforce",
                                    "fork->chroot", error,
                                    "RETURN=-1,ERRNO=1(EPERM)");
    }

    child_pid = fork();
    if (child_pid == 0) {
        close(pipe_fds[0]);
        errno = 0;
        child_result.ret = chroot(dir);
        child_result.error = errno;
        if (write(pipe_fds[1], &child_result, sizeof(child_result)) !=
            (ssize_t)sizeof(child_result)) {
            _exit(1);
        }
        _exit(child_result.ret == -1 && child_result.error == EPERM ? 0 : 1);
    }

    close(pipe_fds[1]);
    if (child_pid == -1 ||
        read(pipe_fds[0], &child_result, sizeof(child_result)) !=
            (ssize_t)sizeof(child_result) ||
        waitpid(child_pid, &status, 0) == -1) {
        close(pipe_fds[0]);
        if (child_pid > 0) {
            (void)waitpid(child_pid, NULL, 0);
        }
        rmdir(dir);
        return report_setup_failure("fork-inheritance", executable,
                                    ENFORCE_PROFILE, "enforce",
                                    "fork->chroot", ECHILD,
                                    "RETURN=-1,ERRNO=1(EPERM)");
    }
    close(pipe_fds[0]);
    rmdir(dir);
    return report("fork-inheritance", executable, ENFORCE_PROFILE, "enforce",
                  "fork->chroot", child_result.ret, child_result.error, -1,
                  EPERM, "RETURN=-1,ERRNO=1(EPERM)");
}

static int run_ptrace(const char *executable)
{
    pid_t child_pid = fork();
    int status;
    long ret;
    int error;

    if (child_pid == 0) {
        execl(ENFORCE_DENY_PATH, ENFORCE_DENY_PATH, "--ptrace-target",
              (char *)NULL);
        _exit(127);
    }
    if (child_pid == -1 ||
        waitpid(child_pid, &status, WUNTRACED) == -1 ||
        !WIFSTOPPED(status)) {
        if (child_pid > 0) {
            (void)kill(child_pid, SIGKILL);
            (void)waitpid(child_pid, NULL, 0);
        }
        int result = report_setup_failure(
            "cross-profile-ptrace-deny", executable, DEFAULT_PROFILE, "enforce",
            "ptrace(PTRACE_ATTACH)", ECHILD, "RETURN=-1,ERRNO=1(EPERM)");
        printf("TRACER_EXPECTED_PROFILE=%s\n", DEFAULT_PROFILE);
        printf("TARGET_EXPECTED_PROFILE=%s\n", ENFORCE_PROFILE);
        return result;
    }

    errno = 0;
    ret = ptrace(PTRACE_ATTACH, child_pid, NULL, NULL);
    error = errno;
    if (ret == 0) {
        (void)ptrace(PTRACE_DETACH, child_pid, NULL, NULL);
    }
    (void)kill(child_pid, SIGKILL);
    (void)waitpid(child_pid, NULL, 0);
    {
        int result = report("cross-profile-ptrace-deny", executable,
                            DEFAULT_PROFILE, "enforce",
                            "ptrace(PTRACE_ATTACH)", (int)ret, error, -1,
                            EPERM, "RETURN=-1,ERRNO=1(EPERM)");
        printf("TRACER_EXPECTED_PROFILE=%s\n", DEFAULT_PROFILE);
        printf("TARGET_EXPECTED_PROFILE=%s\n", ENFORCE_PROFILE);
        return result;
    }
}

int main(int argc, char **argv)
{
    const char *executable = argv[0];
    const char *name = entry_name(executable);

    setvbuf(stdout, NULL, _IONBF, 0);
    if (argc == 2 && strcmp(argv[1], "--ptrace-target") == 0) {
        raise(SIGSTOP);
        pause();
        return 0;
    }
    if (strcmp(name, "apparmor-unconfined") == 0) {
        return run_chroot("unconfined-baseline", executable, DEFAULT_PROFILE,
                          "unconfined", 0, 0, "RETURN=0,ERRNO=0");
    }
    if (strcmp(name, "apparmor-enforce-deny") == 0) {
        return run_chroot("enforce-capability-deny", executable,
                          ENFORCE_PROFILE, "enforce", -1, EPERM,
                          "RETURN=-1,ERRNO=1(EPERM)");
    }
    if (strcmp(name, "apparmor-enforce-allow") == 0) {
        return run_fchown(executable);
    }
    if (strcmp(name, "apparmor-fork") == 0) {
        return run_fork(executable);
    }
    if (strcmp(name, "apparmor-complain") == 0) {
        return run_chroot("complain-capability-report", executable,
                          COMPLAIN_PROFILE, "complain", 0, 0,
                          "RETURN=0,ERRNO=0");
    }
    if (strcmp(name, "apparmor-ptrace") == 0) {
        return run_ptrace(executable);
    }

    fprintf(stderr, "unknown AppArmor manual entry: %s\n", executable);
    return 2;
}
