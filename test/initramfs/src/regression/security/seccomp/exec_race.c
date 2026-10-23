// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"

#include <sys/stat.h>

static pthread_barrier_t barrier;
static char executable[128];
static int sync_before_exec;
static int exec_wins, kill_wins;
static long sync_delay_ns;

static void rendezvous(void)
{
	CHECK_WITH(pthread_barrier_wait(&barrier),
		   _ret == 0 || _ret == PTHREAD_BARRIER_SERIAL_THREAD);
}

/* Run before the test constructors, including in a set-user-ID executable. */
__attribute__((constructor(101))) static void exec_probe(void)
{
	const char *probe = getenv("SECCOMP_EXEC_RACE_PROBE");
	if (!probe)
		return;
	if (!strcmp(probe, "credentials")) {
		int nnp = CHECK(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0));
		CHECK_WITH(getuid(), _ret == 1000);
		CHECK_WITH(geteuid(), _ret == (nnp ? 1000 : 0));
		CHECK_WITH(prctl(PR_GET_SECCOMP),
			   _ret == (nnp ? SECCOMP_MODE_FILTER : 0));
		if (nnp)
			CHECK_WITH(syscall(SYS_getppid),
				   _ret == -1 && errno == EBUSY);
		else
			CHECK(syscall(SYS_getppid));
		fprintf(stderr, "setuid exec: no_new_privs=%d euid=%d\n", nnp,
			geteuid());
	}
	_exit(0);
}

static void exec_self(const char *probe)
{
	char env[80];
	CHECK(snprintf(env, sizeof(env), "SECCOMP_EXEC_RACE_PROBE=%s", probe));
	char *const argv[] = { executable, NULL };
	char *const envp[] = { env, NULL };
	CHECK(execve(executable, argv, envp));
	_exit(1);
}

static void *sync_worker(void *unused)
{
	(void)unused;
	no_new_privs();
	struct sock_filter code[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
			 offsetof(struct seccomp_data, nr)),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_getppid, 0, 1),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | EBUSY),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog prog = { .len = 4, .filter = code };
	rendezvous();
	if (sync_delay_ns) {
		struct timespec delay = { .tv_nsec = sync_delay_ns };
		nanosleep(&delay, NULL);
	}
	CHECK_WITH(syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER,
			   SECCOMP_FILTER_FLAG_TSYNC, &prog),
		   _ret == 0);
	if (sync_before_exec)
		rendezvous();
	return NULL;
}

static void credential_race(void)
{
	CHECK(setresgid(1000, 1000, 1000));
	CHECK(setresuid(1000, 1000, 1000));
	CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 0);
	pthread_t thread;
	CHECK(pthread_barrier_init(&barrier, NULL, 2));
	CHECK(pthread_create(&thread, NULL, sync_worker, NULL));
	rendezvous();
	if (sync_before_exec) {
		rendezvous();
		CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 1);
	}
	exec_self("credentials");
}

static void setuid_baseline(void)
{
	CHECK(setresgid(1000, 1000, 1000));
	CHECK(setresuid(1000, 1000, 1000));
	exec_self("credentials");
}

static void *kill_worker(void *delay_ns)
{
	no_new_privs();
	deny_syscall(SYS_getppid, SECCOMP_RET_KILL_PROCESS);
	rendezvous();
	if (delay_ns) {
		struct timespec delay = { .tv_nsec = (long)delay_ns };
		nanosleep(&delay, NULL);
	}
	syscall(SYS_getppid);
	_exit(1);
}

/* Either group exit wins with SIGSYS, or exec wins and removes its sibling. */
static int kill_exec_race(long delay_ns)
{
	fflush(NULL);
	pid_t child = CHECK(fork());
	if (!child) {
		alarm(10);
		pthread_t thread;
		CHECK(pthread_barrier_init(&barrier, NULL, 2));
		CHECK(pthread_create(&thread, NULL, kill_worker,
				     (void *)delay_ns));
		rendezvous();
		exec_self("exit");
	}
	int status;
	CHECK_WITH(waitpid(child, &status, 0), _ret == child);
	if (WIFEXITED(status) && WEXITSTATUS(status) == 0)
		++exec_wins;
	if (WIFSIGNALED(status) && WTERMSIG(status) == SIGSYS)
		++kill_wins;
	return !((WIFEXITED(status) && WEXITSTATUS(status) == 0) ||
		 (WIFSIGNALED(status) && WTERMSIG(status) == SIGSYS));
}

FN_SETUP(copy_setuid_executable)
{
	/* No ptrace: tracing can itself suppress Linux set-user-ID transitions. */
	CHECK_WITH(geteuid(), _ret == 0);
	CHECK(snprintf(executable, sizeof(executable),
		       "/tmp/seccomp-exec-race-%d", getpid()));
	int src = CHECK(open("/proc/self/exe", O_RDONLY));
	int dst = CHECK(open(executable, O_WRONLY | O_CREAT | O_EXCL, 0700));
	char buf[8192];
	ssize_t count;
	while ((count = CHECK(read(src, buf, sizeof(buf)))) > 0) {
		ssize_t written = 0;
		while (written < count)
			written += CHECK_WITH(write(dst, buf + written,
						    count - written),
					      _ret > 0);
	}
	CHECK(fchmod(dst, 04755));
	CHECK(close(dst));
	CHECK(close(src));
}
END_SETUP()

FN_TEST(exec_credentials_and_group_exit)
{
	alarm(180);
	TEST_RES(run_child(setuid_baseline, 0), _ret == 0);
	sync_before_exec = 1;
	TEST_RES(run_child(credential_race, 0), _ret == 0);
	sync_before_exec = 0;
	const long delays[] = { 0, 100000, 1000000, 10000000 };
	for (int i = 0; i < 4; ++i) {
		for (size_t j = 0; j < sizeof(delays) / sizeof(delays[0]);
		     ++j) {
			sync_delay_ns = delays[j];
			TEST_RES(run_child(credential_race, 0), _ret == 0);
		}
	}
	/* Vary the start offset; these remain bounded scheduling races. */
	for (int i = 0; i < 8; ++i) {
		TEST_RES(kill_exec_race(0), _ret == 0);
		TEST_RES(kill_exec_race(100000), _ret == 0);
		TEST_RES(kill_exec_race(1000000), _ret == 0);
		TEST_RES(kill_exec_race(10000000), _ret == 0);
	}
	fprintf(stderr, "exec/group-exit race: exec=%d SIGSYS=%d\n", exec_wins,
		kill_wins);
	alarm(0);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(unlink(executable));
}
END_SETUP()
