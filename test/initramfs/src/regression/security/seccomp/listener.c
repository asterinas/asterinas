// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"
#include "seccomp_fd_pass.h"
#include <poll.h>
#include <sys/ioctl.h>
#include <sys/resource.h>
#include <sys/socket.h>

static int new_listener(unsigned int flags)
{
	struct sock_filter allow = BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW);
	struct sock_fprog prog = { .len = 1, .filter = &allow };
	return syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER,
		       flags | SECCOMP_FILTER_FLAG_NEW_LISTENER, &prog);
}

FN_SETUP(listener_exec_child)
{
	const char *value = getenv("SECCOMP_LISTENER_EXEC_FD");
	if (!value)
		return;
	int fd = atoi(value);
	CHECK_WITH(fcntl(fd, F_GETFD), _ret == -1 && errno == EBADF);
	CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == SECCOMP_MODE_FILTER);
	fd = CHECK(new_listener(0));
	CHECK(close(fd));
	_exit(0);
}
END_SETUP()

static void exec_closes_listener_but_keeps_filter(void)
{
	no_new_privs();
	int fd = CHECK(new_listener(0));
	char number[32];
	snprintf(number, sizeof(number), "%d", fd);
	CHECK(setenv("SECCOMP_LISTENER_EXEC_FD", number, 1));
	CHECK(execl("/proc/self/exe", "listener", NULL));
	_exit(1);
}

static pthread_barrier_t barrier;

static void rendezvous(void)
{
	CHECK_WITH(pthread_barrier_wait(&barrier),
		   _ret == 0 || _ret == PTHREAD_BARRIER_SERIAL_THREAD);
}

static void *divergent_thread(void *arg)
{
	(void)arg;
	struct sock_filter allow = BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW);
	CHECK(install(&allow, 1));
	rendezvous();
	rendezvous();
	return NULL;
}

static void failed_tsync_releases_reserved_fd(void)
{
	no_new_privs();
	int available = CHECK(dup(STDOUT_FILENO));
	CHECK(close(available));
	CHECK(pthread_barrier_init(&barrier, NULL, 2));
	pthread_t thread;
	CHECK(pthread_create(&thread, NULL, divergent_thread, NULL));
	rendezvous();
	CHECK_WITH(new_listener(SECCOMP_FILTER_FLAG_TSYNC |
				SECCOMP_FILTER_FLAG_TSYNC_ESRCH),
		   _ret == -1 && errno == ESRCH);
	CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == SECCOMP_MODE_DISABLED);
	int next = CHECK(dup(STDOUT_FILENO));
	CHECK_WITH(next, _ret == available);
	CHECK(close(next));
	rendezvous();
	CHECK(pthread_join(thread, NULL));
	CHECK(pthread_barrier_destroy(&barrier));
	int fd = CHECK(new_listener(0));
	CHECK_WITH(fd, _ret == available);
	CHECK(close(fd));
}

static void *synced_thread(void *arg)
{
	(void)arg;
	rendezvous();
	rendezvous();
	CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == SECCOMP_MODE_FILTER);
	CHECK_WITH(new_listener(0), _ret == -1 && errno == EBUSY);
	rendezvous();
	rendezvous();
	int fd = CHECK(new_listener(0));
	CHECK(close(fd));
	return NULL;
}

static void tsync_propagates_listener_ownership(void)
{
	no_new_privs();
	CHECK(pthread_barrier_init(&barrier, NULL, 2));
	pthread_t thread;
	CHECK(pthread_create(&thread, NULL, synced_thread, NULL));
	rendezvous();
	int fd = CHECK(new_listener(SECCOMP_FILTER_FLAG_TSYNC |
				    SECCOMP_FILTER_FLAG_TSYNC_ESRCH |
				    SECCOMP_FILTER_FLAG_LOG));
	rendezvous();
	rendezvous();
	CHECK(close(fd));
	rendezvous();
	CHECK(pthread_join(thread, NULL));
	CHECK(pthread_barrier_destroy(&barrier));
}

static void descriptor_properties(void)
{
	no_new_privs();
	int fd = CHECK(new_listener(0));
	CHECK_WITH(fcntl(fd, F_GETFD), _ret == FD_CLOEXEC);
	CHECK_WITH(fcntl(fd, F_GETFL), (_ret & O_ACCMODE) == O_RDWR);
	char path[64], name[64] = { 0 };
	snprintf(path, sizeof(path), "/proc/self/fd/%d", fd);
	CHECK_WITH(readlink(path, name, sizeof(name) - 1), _ret > 0);
	CHECK_WITH(strcmp(name, "anon_inode:seccomp notify"), _ret == 0);
	char byte = 0;
	CHECK_WITH(read(fd, &byte, 1), _ret == -1 && errno == EINVAL);
	CHECK_WITH(write(fd, &byte, 1), _ret == -1 && errno == EINVAL);
	CHECK_WITH(lseek(fd, 0, SEEK_SET), _ret == -1 && errno == ESPIPE);
	CHECK_WITH(ioctl(fd, 0x1234, 0), _ret == -1 && errno == EINVAL);
	struct pollfd pfd = { .fd = fd, .events = POLLIN | POLLOUT };
	CHECK_WITH(poll(&pfd, 1, 0), _ret == 0);
	CHECK(close(fd));
}

static void closing_last_duplicate_allows_new_listener(void)
{
	no_new_privs();
	int fd = CHECK(new_listener(0));
	int copy = CHECK(dup(fd));
	CHECK_WITH(new_listener(0), _ret == -1 && errno == EBUSY);
	CHECK(close(fd));
	CHECK_WITH(new_listener(0), _ret == -1 && errno == EBUSY);
	CHECK(close(copy));
	fd = CHECK(new_listener(SECCOMP_FILTER_FLAG_LOG));
	CHECK(close(fd));
}

static void listener_flag_combinations(void)
{
	no_new_privs();
	CHECK_WITH(new_listener(SECCOMP_FILTER_FLAG_TSYNC),
		   _ret == -1 && errno == EINVAL);
	CHECK_WITH(syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER,
			   SECCOMP_FILTER_FLAG_NEW_LISTENER |
				   SECCOMP_FILTER_FLAG_TSYNC,
			   NULL),
		   _ret == -1 && errno == EINVAL);
	CHECK_WITH(new_listener(0x80000000), _ret == -1 && errno == EINVAL);
	CHECK_WITH(syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER,
			   SECCOMP_FILTER_FLAG_NEW_LISTENER, NULL),
		   _ret == -1 && errno == EFAULT);
	unsigned int flags[] = {
		0,
		SECCOMP_FILTER_FLAG_SPEC_ALLOW,
		SECCOMP_FILTER_FLAG_LOG,
		SECCOMP_FILTER_FLAG_TSYNC_ESRCH,
		SECCOMP_FILTER_FLAG_TSYNC | SECCOMP_FILTER_FLAG_TSYNC_ESRCH,
		SECCOMP_FILTER_FLAG_LOG | SECCOMP_FILTER_FLAG_TSYNC |
			SECCOMP_FILTER_FLAG_TSYNC_ESRCH,
	};
	for (size_t i = 0; i < sizeof(flags) / sizeof(flags[0]); ++i) {
		int fd = CHECK(new_listener(flags[i]));
		CHECK(close(fd));
	}
}

static void fd_exhaustion_preserves_policy(void)
{
	no_new_privs();
	struct rlimit original;
	CHECK(getrlimit(RLIMIT_NOFILE, &original));
	struct rlimit limit = original;
	limit.rlim_cur = 0;
	CHECK(setrlimit(RLIMIT_NOFILE, &limit));
	CHECK_WITH(new_listener(0), _ret == -1 && errno == EMFILE);
	CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == SECCOMP_MODE_DISABLED);
	CHECK_WITH(syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER,
			   SECCOMP_FILTER_FLAG_NEW_LISTENER, NULL),
		   _ret == -1 && errno == EFAULT);
	CHECK(setrlimit(RLIMIT_NOFILE, &original));
	int fd = CHECK(new_listener(0));
	CHECK(close(fd));
}

static void listener_permission(void)
{
	if (geteuid() == 0)
		CHECK(setuid(65534));
	CHECK_WITH(new_listener(0), _ret == -1 && errno == EACCES);
	no_new_privs();
	int fd = CHECK(new_listener(0));
	CHECK(close(fd));
}

static void listener_hup_after_last_filter_user(void)
{
	int sockets[2], gate[2];
	CHECK(socketpair(AF_UNIX, SOCK_STREAM, 0, sockets));
	CHECK(pipe(gate));
	pid_t child = CHECK(fork());
	if (!child) {
		CHECK(close(sockets[0]));
		CHECK(close(gate[1]));
		no_new_privs();
		int fd = CHECK(new_listener(0));
		send_fd(sockets[1], fd);
		CHECK(close(fd));
		char byte;
		CHECK_WITH(read(gate[0], &byte, 1), _ret == 1);
		_exit(0);
	}
	CHECK(close(sockets[1]));
	CHECK(close(gate[0]));
	int fd = receive_fd(sockets[0]);
	struct pollfd pfd = { .fd = fd, .events = POLLIN | POLLOUT };
	CHECK_WITH(poll(&pfd, 1, 0), _ret == 0);
	CHECK_WITH(write(gate[1], "x", 1), _ret == 1);
	CHECK_WITH(poll(&pfd, 1, 5000), _ret == 1 && pfd.revents == POLLHUP);
	int status;
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFEXITED(status) &&
			   WEXITSTATUS(status) == 0);
	CHECK(close(fd));
	CHECK(close(sockets[0]));
	CHECK(close(gate[1]));
}

static void *last_listener_thread(void *arg)
{
	int *fd = arg;
	no_new_privs();
	*fd = CHECK(new_listener(0));
	return NULL;
}

static void listener_hup_after_join(void)
{
	int fd;
	pthread_t thread;
	CHECK(pthread_create(&thread, NULL, last_listener_thread, &fd));
	CHECK(pthread_join(thread, NULL));
	struct pollfd pfd = { .fd = fd, .events = POLLIN | POLLOUT };
	CHECK_WITH(poll(&pfd, 1, 0), _ret == 1 && pfd.revents == POLLHUP);
	CHECK(close(fd));
}

FN_TEST(listener)
{
	TEST_RES(run_child(descriptor_properties, 0), _ret == 0);
	TEST_RES(run_child(closing_last_duplicate_allows_new_listener, 0),
		 _ret == 0);
	TEST_RES(run_child(listener_flag_combinations, 0), _ret == 0);
	TEST_RES(run_child(fd_exhaustion_preserves_policy, 0), _ret == 0);
	TEST_RES(run_child(listener_permission, 0), _ret == 0);
	TEST_RES(run_child(listener_hup_after_last_filter_user, 0), _ret == 0);
	TEST_RES(run_child(listener_hup_after_join, 0), _ret == 0);
	TEST_RES(run_child(exec_closes_listener_but_keeps_filter, 0),
		 _ret == 0);
	TEST_RES(run_child(failed_tsync_releases_reserved_fd, 0), _ret == 0);
	TEST_RES(run_child(tsync_propagates_listener_ownership, 0), _ret == 0);
}
END_TEST()
