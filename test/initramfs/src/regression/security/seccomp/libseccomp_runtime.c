// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"
#include <seccomp.h>
#include <linux/capability.h>

static pthread_barrier_t barrier;
static int divergent;
static int shared_base = 1;
static volatile sig_atomic_t trapped;

static scmp_filter_ctx deny_context(int nr, int error)
{
	scmp_filter_ctx ctx =
		CHECK_WITH(seccomp_init(SCMP_ACT_ALLOW), _ret != NULL);
	CHECK(seccomp_rule_add(ctx, SCMP_ACT_ERRNO(error), nr, 0));
	return ctx;
}

static void load_deny(int nr, int error)
{
	scmp_filter_ctx ctx = deny_context(nr, error);
	CHECK(seccomp_load(ctx));
	seccomp_release(ctx);
}

static void automatic_nnp(void)
{
	CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 0);
	load_deny(SCMP_SYS(getppid), EBUSY);
	CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 1);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
}

static void explicit_nnp(void)
{
	no_new_privs();
	scmp_filter_ctx ctx = deny_context(SCMP_SYS(getppid), EACCES);
	CHECK(seccomp_attr_set(ctx, SCMP_FLTATR_CTL_NNP, 0));
	CHECK(seccomp_load(ctx));
	seccomp_release(ctx);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EACCES);
}

static void missing_permission(void)
{
	/* Drop every capability to test authorization even in a root guest. */
	struct __user_cap_header_struct header = {
		.version = _LINUX_CAPABILITY_VERSION_3
	};
	struct __user_cap_data_struct data[2] = { 0 };
	CHECK(syscall(SYS_capset, &header, data));
	CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 0);
	scmp_filter_ctx ctx = deny_context(SCMP_SYS(getppid), EBUSY);
	CHECK(seccomp_attr_set(ctx, SCMP_FLTATR_CTL_NNP, 0));
	CHECK_WITH(seccomp_load(ctx), _ret == -ECANCELED);
	CHECK(seccomp_attr_set(ctx, SCMP_FLTATR_API_SYSRAWRC, 1));
	CHECK_WITH(seccomp_load(ctx), _ret == -EACCES);
	CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == SECCOMP_MODE_DISABLED);
	CHECK(syscall(SYS_getppid));
	seccomp_release(ctx);
}

static void failed_load_preserves_policy(void)
{
	/* Warm library detection before the old policy starts denying installs. */
	CHECK_WITH(seccomp_api_get(), _ret >= 2);
	scmp_filter_ctx old = deny_context(SCMP_SYS(getppid), EBUSY);
	CHECK(seccomp_rule_add(old, SCMP_ACT_ERRNO(EPERM), SCMP_SYS(seccomp),
			       0));
	CHECK(seccomp_load(old));
	seccomp_release(old);
	scmp_filter_ctx ctx = deny_context(SCMP_SYS(getuid), EACCES);
	CHECK_WITH(seccomp_load(ctx), _ret == -ECANCELED);
	CHECK(seccomp_attr_set(ctx, SCMP_FLTATR_API_SYSRAWRC, 1));
	/* libseccomp 2.6.0 maps EPERM to EFAULT even with SYSRAWRC. */
	int raw_result = seccomp_load(ctx);
	printf("LIBSECCOMP_LOAD denied_install raw=%d\n", raw_result);
	fflush(stdout);
	CHECK_WITH(raw_result, _ret == -EFAULT);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
	CHECK(syscall(SYS_getuid));
	seccomp_release(ctx);
}

static void repeated_loads_only_restrict(void)
{
	load_deny(SCMP_SYS(getppid), EBUSY);
	scmp_filter_ctx ctx =
		CHECK_WITH(seccomp_init(SCMP_ACT_ALLOW), _ret != NULL);
	CHECK(seccomp_load(ctx));
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
	CHECK(seccomp_rule_add_exact(ctx, SCMP_ACT_ERRNO(EACCES),
				     SCMP_SYS(getuid), 0));
	CHECK(seccomp_load(ctx));
	seccomp_release(ctx);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
	CHECK_WITH(syscall(SYS_getuid), _ret == -1 && errno == EACCES);
	CHECK(syscall(SYS_getgid));
}

static void fork_inherits(void)
{
	load_deny(SCMP_SYS(getppid), EBUSY);
	pid_t child = CHECK(fork());
	if (!child) {
		CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
		load_deny(SCMP_SYS(getuid), EACCES);
		CHECK_WITH(syscall(SYS_getuid), _ret == -1 && errno == EACCES);
		_exit(0);
	}
	int status;
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFEXITED(status) &&
			   WEXITSTATUS(status) == 0);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
	CHECK(syscall(SYS_getuid));
}

static void exec_inherits(void)
{
	CHECK(setenv("LIBSECCOMP_EXEC_PROBE", "1", 1));
	load_deny(SCMP_SYS(getppid), EBUSY);
	execl("/proc/self/exe", "libseccomp_runtime", NULL);
	_exit(90);
}

static void rendezvous(void)
{
	CHECK_WITH(pthread_barrier_wait(&barrier),
		   _ret == 0 || _ret == PTHREAD_BARRIER_SERIAL_THREAD);
}

static void *sync_target(void *arg)
{
	int own_divergence = arg != NULL;
	if (!shared_base)
		CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 0);
	if (own_divergence)
		load_deny(SCMP_SYS(getuid), EACCES);
	rendezvous();
	rendezvous();
	if (shared_base)
		CHECK_WITH(syscall(SYS_getgid), _ret == -1 && errno == EPERM);
	else
		CHECK(syscall(SYS_getgid));
	if (divergent) {
		CHECK(syscall(SYS_getppid));
		if (own_divergence)
			CHECK_WITH(syscall(SYS_getuid),
				   _ret == -1 && errno == EACCES);
		else
			CHECK(syscall(SYS_getuid));
	} else {
		CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
		CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 1);
	}
	return NULL;
}

static void sync_threads(void)
{
	if (shared_base)
		load_deny(SCMP_SYS(getgid), EPERM);
	CHECK(pthread_barrier_init(&barrier, NULL, divergent ? 3 : 2));
	pthread_t worker, untouched;
	if (divergent)
		CHECK(pthread_create(&untouched, NULL, sync_target, NULL));
	CHECK(pthread_create(&worker, NULL, sync_target,
			     divergent ? (void *)1 : NULL));
	rendezvous();
	scmp_filter_ctx ctx = deny_context(SCMP_SYS(getppid), EBUSY);
	CHECK(seccomp_attr_set(ctx, SCMP_FLTATR_CTL_TSYNC, 1));
	int result = seccomp_load(ctx);
	/* CTL_TSYNC probes TSYNC_ESRCH independently of the overall API level. */
	int expected = divergent ? -ECANCELED : 0;
	printf("LIBSECCOMP_LOAD tsync divergent=%d result=%d expected=%d\n",
	       divergent, result, expected);
	fflush(stdout);
	CHECK_WITH(result, _ret == expected);
	if (divergent) {
		CHECK(seccomp_attr_set(ctx, SCMP_FLTATR_API_SYSRAWRC, 1));
		int raw_result = seccomp_load(ctx);
		printf("LIBSECCOMP_LOAD tsync_raw=%d\n", raw_result);
		fflush(stdout);
		CHECK_WITH(raw_result, _ret == -ESRCH);
	}
	seccomp_release(ctx);
	if (divergent) {
		CHECK(syscall(SYS_getppid));
		CHECK(syscall(SYS_getuid));
	} else {
		CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
	}
	if (shared_base)
		CHECK_WITH(syscall(SYS_getgid), _ret == -1 && errno == EPERM);
	else
		CHECK(syscall(SYS_getgid));
	rendezvous();
	CHECK(pthread_join(worker, NULL));
	if (divergent)
		CHECK(pthread_join(untouched, NULL));
	CHECK(pthread_barrier_destroy(&barrier));
}

static void sync_success(void)
{
	divergent = 0;
	sync_threads();
}

static void sync_failure(void)
{
	divergent = 1;
	sync_threads();
}

static void sync_disabled_threads(void)
{
	shared_base = 0;
	divergent = 0;
	sync_threads();
}

static void capability_detection(void)
{
	unsigned int api = seccomp_api_get();
	CHECK_WITH(api, _ret >= 3);
	scmp_filter_ctx ctx =
		CHECK_WITH(seccomp_init(SCMP_ACT_ALLOW), _ret != NULL);
	CHECK(seccomp_attr_set(ctx, SCMP_FLTATR_CTL_TSYNC, 1));
	CHECK(seccomp_attr_set(ctx, SCMP_FLTATR_CTL_LOG, 1));
	int ssb = seccomp_attr_set(ctx, SCMP_FLTATR_CTL_SSB, 1);
	CHECK_WITH(ssb, api >= 4 ? _ret == 0 : _ret == -EOPNOTSUPP);
	printf("LIBSECCOMP_DETECTION api=%u tsync=0 log=0 ssb=%d\n", api, ssb);
	fflush(stdout);
	seccomp_release(ctx);
}

static void *isolated_worker(void *unused)
{
	(void)unused;
	rendezvous();
	rendezvous();
	CHECK(syscall(SYS_getppid));
	CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == SECCOMP_MODE_DISABLED);
	return NULL;
}

static void ordinary_load_is_thread_local(void)
{
	CHECK(pthread_barrier_init(&barrier, NULL, 2));
	pthread_t worker;
	CHECK(pthread_create(&worker, NULL, isolated_worker, NULL));
	rendezvous();
	load_deny(SCMP_SYS(getppid), EBUSY);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
	rendezvous();
	CHECK(pthread_join(worker, NULL));
	CHECK(pthread_barrier_destroy(&barrier));
}

static void write_has_no_side_effect(void)
{
	int pipefd[2];
	CHECK(pipe(pipefd));
	CHECK(fcntl(pipefd[0], F_SETFL, O_NONBLOCK));
	scmp_filter_ctx ctx =
		CHECK_WITH(seccomp_init(SCMP_ACT_ALLOW), _ret != NULL);
	CHECK(seccomp_rule_add(ctx, SCMP_ACT_ERRNO(EACCES), SCMP_SYS(write), 2,
			       SCMP_A0_32(SCMP_CMP_EQ, pipefd[1]),
			       SCMP_A2_64(SCMP_CMP_EQ, 1)));
	CHECK(seccomp_load(ctx));
	seccomp_release(ctx);
	CHECK_WITH(write(pipefd[1], "x", 1), _ret == -1 && errno == EACCES);
	char bytes[2];
	CHECK_WITH(read(pipefd[0], bytes, sizeof(bytes)),
		   _ret == -1 && errno == EAGAIN);
	CHECK_WITH(write(pipefd[1], "ok", 2), _ret == 2);
	CHECK_WITH(read(pipefd[0], bytes, sizeof(bytes)),
		   _ret == 2 && memcmp(bytes, "ok", 2) == 0);
	CHECK(close(pipefd[0]));
	CHECK(close(pipefd[1]));
}

static void trap_handler(int signal, siginfo_t *info, void *unused)
{
	(void)unused;
	if (signal != SIGSYS || info->si_code != 1 ||
	    info->si_syscall != SYS_getppid || info->si_arch != NATIVE_ARCH)
		_exit(92);
	++trapped;
}

static void handled_trap(void)
{
	struct sigaction action = { .sa_sigaction = trap_handler,
				    .sa_flags = SA_SIGINFO };
	CHECK(sigemptyset(&action.sa_mask));
	CHECK(sigaction(SIGSYS, &action, NULL));
	scmp_filter_ctx ctx =
		CHECK_WITH(seccomp_init(SCMP_ACT_ALLOW), _ret != NULL);
	CHECK(seccomp_rule_add(ctx, SCMP_ACT_TRAP, SCMP_SYS(getppid), 0));
	CHECK(seccomp_load(ctx));
	seccomp_release(ctx);
	syscall(SYS_getppid);
	CHECK_WITH(trapped, _ret == 1);
	CHECK(syscall(SYS_getpid));
}

static void *fatal_worker(void *unused)
{
	(void)unused;
	syscall(SYS_getppid);
	_exit(93);
}

static void kill_thread_keeps_sibling(void)
{
	scmp_filter_ctx ctx =
		CHECK_WITH(seccomp_init(SCMP_ACT_ALLOW), _ret != NULL);
	CHECK(seccomp_rule_add(ctx, SCMP_ACT_KILL_THREAD, SCMP_SYS(getppid),
			       0));
	CHECK(seccomp_load(ctx));
	seccomp_release(ctx);
	pthread_t worker;
	CHECK(pthread_create(&worker, NULL, fatal_worker, NULL));
	CHECK(pthread_join(worker, NULL));
	CHECK(syscall(SYS_getpid));
}

static void kill_process_ends_siblings(void)
{
	scmp_filter_ctx ctx =
		CHECK_WITH(seccomp_init(SCMP_ACT_ALLOW), _ret != NULL);
	CHECK(seccomp_rule_add(ctx, SCMP_ACT_KILL_PROCESS, SCMP_SYS(getppid),
			       0));
	CHECK(seccomp_load(ctx));
	seccomp_release(ctx);
	pthread_t worker;
	CHECK(pthread_create(&worker, NULL, fatal_worker, NULL));
	CHECK(pthread_join(worker, NULL));
	_exit(94);
}

FN_TEST(libseccomp_runtime)
{
	if (getenv("LIBSECCOMP_EXEC_PROBE")) {
		CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
		CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 1);
		_exit(0);
	}
	const struct scmp_version *version = seccomp_version();
	printf("LIBSECCOMP_RUNTIME_VERSION=%u.%u.%u API=%u\n", version->major,
	       version->minor, version->micro, seccomp_api_get());
	TEST_RES(run_child(capability_detection, 0), _ret == 0);
	TEST_RES(run_child(sync_disabled_threads, 0), _ret == 0);
	TEST_RES(run_child(automatic_nnp, 0), _ret == 0);
	TEST_RES(run_child(explicit_nnp, 0), _ret == 0);
	TEST_RES(run_child(missing_permission, 0), _ret == 0);
	TEST_RES(run_child(failed_load_preserves_policy, 0), _ret == 0);
	TEST_RES(run_child(repeated_loads_only_restrict, 0), _ret == 0);
	TEST_RES(run_child(fork_inherits, 0), _ret == 0);
	TEST_RES(run_child(exec_inherits, 0), _ret == 0);
	TEST_RES(run_child(sync_success, 0), _ret == 0);
	TEST_RES(run_child(sync_failure, 0), _ret == 0);
	TEST_RES(run_child(ordinary_load_is_thread_local, 0), _ret == 0);
	TEST_RES(run_child(write_has_no_side_effect, 0), _ret == 0);
	TEST_RES(run_child(handled_trap, 0), _ret == 0);
	TEST_RES(run_child(kill_thread_keeps_sibling, 0), _ret == 0);
	TEST_RES(run_child(kill_process_ends_siblings, SIGSYS), _ret == 0);
}
END_TEST()
