// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"
#include <sys/ptrace.h>

#define SECCOMP_MODE_DEAD 3

static void read_status(int fd, char *buf, size_t capacity, size_t chunk)
{
	size_t used = 0;
	for (;;) {
		CHECK_WITH(used, _ret + chunk < capacity);
		ssize_t len = CHECK(read(fd, buf + used, chunk));
		if (!len)
			break;
		used += len;
	}
	buf[used] = '\0';
}

static void check_status_fields(const char *buf, int nnp, int mode, int filters)
{
	const char *names[] = { "NoNewPrivs:", "Seccomp:", "Seccomp_filters:" };
	int expected[] = { nnp, mode, filters };
	for (size_t i = 0; i < 3; ++i) {
		const char *field =
			CHECK_WITH(strstr(buf, names[i]), _ret != NULL);
		CHECK_WITH(field == buf || field[-1] == '\n', _ret);
		CHECK_WITH(strstr(field + 1, names[i]), _ret == NULL);
		char value[64];
		snprintf(value, sizeof(value), "%s\t%d\n", names[i],
			 expected[i]);
		CHECK_WITH(strncmp(field, value, strlen(value)), _ret == 0);
	}
}

static void check_status(pid_t pid, pid_t tid, int nnp, int mode, int filters,
			 size_t chunk)
{
	char path[128], buf[8192];
	if (tid)
		snprintf(path, sizeof(path), "/proc/%d/task/%d/status", pid,
			 tid);
	else
		snprintf(path, sizeof(path), "/proc/%d/status", pid);
	int fd = CHECK(open(path, O_RDONLY));
	read_status(fd, buf, sizeof(buf), chunk);
	CHECK(close(fd));
	check_status_fields(buf, nnp, mode, filters);
}

static void check_self(int nnp, int mode, int filters)
{
	check_status(getpid(), 0, nnp, mode, filters, 1024);
	check_status(getpid(), syscall(SYS_gettid), nnp, mode, filters, 7);
}

static int allow_filter(unsigned int flags)
{
	struct sock_filter code[] = {
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog prog = { .len = 1, .filter = code };
	return syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER, flags, &prog);
}

static void short_reads_keep_status_snapshot(void)
{
	no_new_privs();
	int fd = CHECK(open("/proc/self/status", O_RDONLY));
	char buf[8192];
	size_t used = 0;
	/* Stop between the mode and filter count before changing the policy. */
	do {
		CHECK_WITH(used, _ret + 1 < sizeof(buf));
		CHECK_WITH(read(fd, buf + used, 1), _ret == 1);
		buf[++used] = '\0';
	} while (!strstr(buf, "\nSeccomp:\t0\n"));
	CHECK(allow_filter(0));
	check_self(1, SECCOMP_MODE_FILTER, 1);

	read_status(fd, buf + used, sizeof(buf) - used, 7);
	check_status_fields(buf, 1, SECCOMP_MODE_DISABLED, 0);
	CHECK_WITH(lseek(fd, 0, SEEK_SET), _ret == 0);
	read_status(fd, buf, sizeof(buf), 7);
	check_status_fields(buf, 1, SECCOMP_MODE_FILTER, 1);
	CHECK(close(fd));
}

static void modes_and_stacking(void)
{
	check_self(0, SECCOMP_MODE_DISABLED, 0);
	no_new_privs();
	check_self(1, SECCOMP_MODE_DISABLED, 0);
	for (int i = 1; i <= 16; ++i) {
		CHECK(allow_filter(0));
		check_self(1, SECCOMP_MODE_FILTER, i);
	}
	/* A missing return is invalid regardless of the current filter chain. */
	struct sock_filter invalid[] = { BPF_STMT(BPF_LD | BPF_IMM, 1) };
	CHECK_WITH(install(invalid, 1), _ret == -1 && errno == EINVAL);
	check_self(1, SECCOMP_MODE_FILTER, 16);
}

static void fork_inherits_status(void)
{
	no_new_privs();
	CHECK(allow_filter(0));
	pid_t child = CHECK(fork());
	if (!child) {
		check_self(1, SECCOMP_MODE_FILTER, 1);
		CHECK(allow_filter(0));
		check_self(1, SECCOMP_MODE_FILTER, 2);
		_exit(0);
	}
	int status;
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFEXITED(status) &&
			   WEXITSTATUS(status) == 0);
	check_self(1, SECCOMP_MODE_FILTER, 1);
}

static pthread_barrier_t barrier;
static pid_t worker_tid;

static void rendezvous(void)
{
	CHECK_WITH(pthread_barrier_wait(&barrier),
		   _ret == 0 || _ret == PTHREAD_BARRIER_SERIAL_THREAD);
}

static void *private_worker(void *unused)
{
	(void)unused;
	worker_tid = syscall(SYS_gettid);
	no_new_privs();
	CHECK(allow_filter(0));
	CHECK(allow_filter(0));
	check_status(getpid(), 0, 0, SECCOMP_MODE_DISABLED, 0, 1024);
	check_status(getpid(), worker_tid, 1, SECCOMP_MODE_FILTER, 2, 7);
	rendezvous();
	rendezvous();
	return NULL;
}

static void thread_specific_status(void)
{
	pthread_t thread;
	CHECK(pthread_barrier_init(&barrier, NULL, 2));
	CHECK(pthread_create(&thread, NULL, private_worker, NULL));
	rendezvous();
	check_self(0, SECCOMP_MODE_DISABLED, 0);
	check_status(getpid(), worker_tid, 1, SECCOMP_MODE_FILTER, 2, 1024);
	/* Independently installed identical filters are not TSYNC ancestors. */
	no_new_privs();
	CHECK(allow_filter(0));
	CHECK_WITH(allow_filter(SECCOMP_FILTER_FLAG_TSYNC), _ret == worker_tid);
	check_self(1, SECCOMP_MODE_FILTER, 1);
	check_status(getpid(), worker_tid, 1, SECCOMP_MODE_FILTER, 2, 1024);
	rendezvous();
	CHECK(pthread_join(thread, NULL));
	CHECK(pthread_barrier_destroy(&barrier));
}

static void *sync_worker(void *unused)
{
	(void)unused;
	worker_tid = syscall(SYS_gettid);
	rendezvous();
	rendezvous();
	check_self(1, SECCOMP_MODE_FILTER, 2);
	CHECK(allow_filter(0));
	check_status(getpid(), worker_tid, 1, SECCOMP_MODE_FILTER, 3, 7);
	check_status(getpid(), 0, 1, SECCOMP_MODE_FILTER, 2, 1024);
	return NULL;
}

static void synchronized_status(void)
{
	pthread_t thread;
	CHECK(pthread_barrier_init(&barrier, NULL, 2));
	CHECK(pthread_create(&thread, NULL, sync_worker, NULL));
	rendezvous();
	no_new_privs();
	CHECK(allow_filter(0));
	check_status(getpid(), worker_tid, 0, SECCOMP_MODE_DISABLED, 0, 1024);
	CHECK_WITH(allow_filter(SECCOMP_FILTER_FLAG_TSYNC), _ret == 0);
	check_status(getpid(), worker_tid, 1, SECCOMP_MODE_FILTER, 2, 1024);
	rendezvous();
	CHECK(pthread_join(thread, NULL));
	CHECK(pthread_barrier_destroy(&barrier));
	check_self(1, SECCOMP_MODE_FILTER, 2);
}

static void *inherited_worker(void *unused)
{
	(void)unused;
	check_self(1, SECCOMP_MODE_FILTER, 2);
	return NULL;
}

static void clone_inherits_status(void)
{
	no_new_privs();
	CHECK(allow_filter(0));
	CHECK(allow_filter(0));
	pthread_t thread;
	CHECK(pthread_create(&thread, NULL, inherited_worker, NULL));
	CHECK(pthread_join(thread, NULL));
}

__attribute__((constructor(101))) static void exec_probe(void)
{
	if (!getenv("SECCOMP_STATUS_EXEC"))
		return;
	check_self(1, SECCOMP_MODE_FILTER, 2);
	_exit(0);
}

static void exec_with_status(void)
{
	no_new_privs();
	CHECK(allow_filter(0));
	CHECK(allow_filter(0));
	char *const argv[] = { "seccomp-status-exec", NULL };
	char *const envp[] = { "SECCOMP_STATUS_EXEC=1", NULL };
	CHECK(execve("/proc/self/exe", argv, envp));
	_exit(1);
}

static void *exec_worker(void *unused)
{
	(void)unused;
	exec_with_status();
	return NULL;
}

static void nonleader_exec_status(void)
{
	pthread_t thread;
	CHECK(pthread_create(&thread, NULL, exec_worker, NULL));
	CHECK(pthread_join(thread, NULL));
	_exit(1);
}

static void strict_status(void)
{
	int ready[2], release[2];
	CHECK(pipe(ready));
	CHECK(pipe(release));
	pid_t child = CHECK(fork());
	if (!child) {
		CHECK(close(ready[0]));
		CHECK(close(release[1]));
		CHECK(syscall(SYS_seccomp, SECCOMP_SET_MODE_STRICT, 0, NULL));
		char byte = 0;
		/* Strict mode permits only raw read/write/exit here. */
		long written = syscall(SYS_write, ready[1], &byte, 1);
		long read_len = syscall(SYS_read, release[0], &byte, 1);
		syscall(SYS_exit, written == 1 && read_len == 1 ? 0 : 1);
		__builtin_unreachable();
	}
	CHECK(close(ready[1]));
	CHECK(close(release[0]));
	char byte;
	CHECK_WITH(read(ready[0], &byte, 1), _ret == 1);
	check_status(child, 0, 0, SECCOMP_MODE_STRICT, 0, 1024);
	check_status(child, child, 0, SECCOMP_MODE_STRICT, 0, 7);
	check_self(0, SECCOMP_MODE_DISABLED, 0);
	CHECK_WITH(write(release[1], &byte, 1), _ret == 1);
	CHECK(close(ready[0]));
	CHECK(close(release[1]));
	siginfo_t info;
	CHECK(waitid(P_PID, child, &info, WEXITED | WNOWAIT));
	check_status(child, 0, 0, SECCOMP_MODE_STRICT, 0, 1024);
	int status;
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFEXITED(status) &&
			   WEXITSTATUS(status) == 0);
}

static void strict_violation_status(void)
{
	pid_t child = CHECK(fork());
	if (!child) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGSTOP));
		CHECK(syscall(SYS_seccomp, SECCOMP_SET_MODE_STRICT, 0, NULL));
		syscall(SYS_getpid);
		syscall(SYS_exit, 1);
		__builtin_unreachable();
	}
	int status;
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFSTOPPED(status) &&
			   WSTOPSIG(status) == SIGSTOP);
	CHECK(ptrace(PTRACE_SETOPTIONS, child, 0, PTRACE_O_TRACEEXIT));
	CHECK(ptrace(PTRACE_CONT, child, 0, 0));
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFSTOPPED(status) &&
			   (unsigned int)status >> 16 == PTRACE_EVENT_EXIT);
	check_status(child, 0, 0, SECCOMP_MODE_DEAD, 0, 1024);
	CHECK(ptrace(PTRACE_CONT, child, 0, 0));
	siginfo_t info;
	CHECK(waitid(P_PID, child, &info, WEXITED | WNOWAIT));
	check_status(child, 0, 0, SECCOMP_MODE_DEAD, 0, 1024);
	check_status(child, child, 0, SECCOMP_MODE_DEAD, 0, 7);
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFSIGNALED(status) &&
			   WTERMSIG(status) == SIGKILL);
}

static void zombie_status(uint32_t action)
{
	pid_t child = CHECK(fork());
	if (!child) {
		no_new_privs();
		CHECK(allow_filter(0));
		deny_syscall(SYS_getppid, action);
		if (action != SECCOMP_RET_ALLOW) {
			syscall(SYS_getppid);
			_exit(1);
		}
		_exit(0);
	}
	siginfo_t info;
	CHECK(waitid(P_PID, child, &info, WEXITED | WNOWAIT));
	int mode = action == SECCOMP_RET_ALLOW ? SECCOMP_MODE_FILTER :
						 SECCOMP_MODE_DEAD;
	check_status(child, 0, 1, mode, 2, 1024);
	check_status(child, child, 1, mode, 2, 7);
	int status;
	CHECK_WITH(waitpid(child, &status, 0), _ret == child);
	if (action == SECCOMP_RET_ALLOW)
		CHECK_WITH(status, WIFEXITED(_ret) && WEXITSTATUS(_ret) == 0);
	else
		CHECK_WITH(status,
			   WIFSIGNALED(_ret) && WTERMSIG(_ret) == SIGSYS);
}

static void normal_exit_status(void)
{
	zombie_status(SECCOMP_RET_ALLOW);
}

static void kill_thread_status(void)
{
	zombie_status(SECCOMP_RET_KILL_THREAD);
}

static void kill_process_status(void)
{
	zombie_status(SECCOMP_RET_KILL_PROCESS);
}

FN_TEST(proc_status)
{
	TEST_RES(run_child(modes_and_stacking, 0), _ret == 0);
	TEST_RES(run_child(short_reads_keep_status_snapshot, 0), _ret == 0);
	TEST_RES(run_child(fork_inherits_status, 0), _ret == 0);
	TEST_RES(run_child(thread_specific_status, 0), _ret == 0);
	TEST_RES(run_child(synchronized_status, 0), _ret == 0);
	TEST_RES(run_child(clone_inherits_status, 0), _ret == 0);
	TEST_RES(run_child(exec_with_status, 0), _ret == 0);
	TEST_RES(run_child(nonleader_exec_status, 0), _ret == 0);
	TEST_RES(run_child(strict_status, 0), _ret == 0);
	TEST_RES(run_child(strict_violation_status, 0), _ret == 0);
	TEST_RES(run_child(normal_exit_status, 0), _ret == 0);
	TEST_RES(run_child(kill_thread_status, 0), _ret == 0);
	TEST_RES(run_child(kill_process_status, 0), _ret == 0);
}
END_TEST()
