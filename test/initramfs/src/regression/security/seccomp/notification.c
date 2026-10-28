// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"
#include "seccomp_log_capture.h"
#include <poll.h>
#include <sys/ioctl.h>
#include <sys/mman.h>

static int notification_filter(uint32_t action, unsigned int flags)
{
	struct sock_filter code[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
			 offsetof(struct seccomp_data, nr)),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_getppid, 0, 1),
		BPF_STMT(BPF_RET | BPF_K, action),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog prog = { .len = 4, .filter = code };
	return syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER, flags, &prog);
}

static int listener_flags(unsigned int flags)
{
	no_new_privs();
	return CHECK(
		notification_filter(SECCOMP_RET_USER_NOTIF | 0x1234,
				    SECCOMP_FILTER_FLAG_NEW_LISTENER | flags));
}

static int listener(void)
{
	return listener_flags(0);
}

static pid_t notifying_child_logged(int fd, long expected, int expected_errno,
				    int log_count)
{
	pid_t child = CHECK(fork());
	if (child)
		return child;
	CHECK(close(fd));
	alarm(10);
	if (log_count >= 0) {
		printf("LOG_EXPECT pid=%d syscall=%d action=%#x count=%d\n",
		       getpid(), SYS_getppid, SECCOMP_RET_USER_NOTIF,
		       log_count);
		fflush(stdout);
	}
	errno = 0;
	long result = syscall(SYS_getppid, 0x1122334455667788UL, 2UL, 3UL, 4UL,
			      5UL, 0x8877665544332211UL);
	int error = errno;
	if (result != expected || error != expected_errno)
		fprintf(stderr,
			"notification response: got %ld/%d, expected %ld/%d\n",
			result, error, expected, expected_errno);
	CHECK_WITH(result, _ret == expected && error == expected_errno);
	_exit(0);
}

static pid_t notifying_child(int fd, long expected, int expected_errno)
{
	return notifying_child_logged(fd, expected, expected_errno, -1);
}

static void reap(pid_t child)
{
	int status;
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFEXITED(status) &&
			   WEXITSTATUS(status) == 0);
}

static struct seccomp_notif receive(int fd, pid_t child)
{
	struct seccomp_notif request = { 0 };
	CHECK(ioctl(fd, SECCOMP_IOCTL_NOTIF_RECV, &request));
	CHECK_WITH(request.pid, _ret == child);
	CHECK_WITH(request.flags, _ret == 0);
	CHECK_WITH(request.data.nr, _ret == SYS_getppid);
	CHECK_WITH(request.data.arch, _ret == NATIVE_ARCH);
	CHECK_WITH(request.data.instruction_pointer, _ret != 0);
	CHECK_WITH(request.data.args[0], _ret == 0x1122334455667788UL);
	CHECK_WITH(request.data.args[1], _ret == 2);
	CHECK_WITH(request.data.args[5], _ret == 0x8877665544332211UL);
	CHECK(ioctl(fd, SECCOMP_IOCTL_NOTIF_ID_VALID, &request.id));
	CHECK(ioctl(fd, _IOR('!', 2, uint64_t), &request.id));
	return request;
}

static void responses_and_ids(void)
{
	int fd = listener();
	struct {
		int64_t val;
		int32_t error;
		uint32_t flags;
		long expected;
		int expected_errno;
	} cases[] = {
		{ 0x123456789L, 0, 0, 0x123456789L, 0 },
		{ 42, -EPERM, 0, -1, EPERM },
		{ 42, 23, 0, 23, 0 },
		{ 42, INT32_MIN, 0, INT32_MIN, 0 },
		{ -4096, 0, 0, -4096, 0 },
		{ 0, 0, SECCOMP_USER_NOTIF_FLAG_CONTINUE, getpid(), 0 },
	};
	uint64_t previous = 0;
	for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i) {
		pid_t child = notifying_child(fd, cases[i].expected,
					      cases[i].expected_errno);
		struct seccomp_notif request = receive(fd, child);
		if (i)
			CHECK_WITH(request.id != previous, _ret);
		previous = request.id;
		struct seccomp_notif_resp response = {
			.id = request.id,
			.val = cases[i].val,
			.error = cases[i].error,
			.flags = cases[i].flags,
		};
		CHECK(ioctl(fd, SECCOMP_IOCTL_NOTIF_SEND, &response));
		reap(child);
		CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_ID_VALID, &request.id),
			   _ret == -1 && errno == ENOENT);
		CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_SEND, &response),
			   _ret == -1 && errno == ENOENT);
	}
	CHECK(close(fd));
}

static void no_listener_returns_enosys(void)
{
	no_new_privs();
	CHECK(notification_filter(SECCOMP_RET_USER_NOTIF, 0));
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == ENOSYS);
}

static void closed_listener_returns_enosys(void)
{
	int fd = listener();
	CHECK(close(fd));
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == ENOSYS);
}

static void newest_filter_owns_notification(void)
{
	int fd = listener();
	CHECK(notification_filter(SECCOMP_RET_USER_NOTIF, 0));
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == ENOSYS);
	struct pollfd pfd = { .fd = fd, .events = POLLIN | POLLOUT };
	CHECK_WITH(poll(&pfd, 1, 0), _ret == 0);
	CHECK(close(fd));
}

static void errno_precedes_notification(void)
{
	int fd = listener();
	CHECK(notification_filter(SECCOMP_RET_ERRNO | EBUSY, 0));
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
	struct pollfd pfd = { .fd = fd, .events = POLLIN | POLLOUT };
	CHECK_WITH(poll(&pfd, 1, 0), _ret == 0);
	CHECK(close(fd));
}

static void invalid_requests_preserve_notification(void)
{
	int fd = listener();
	struct seccomp_notif dirty = { .flags = 1 };
	CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_RECV, &dirty),
		   _ret == -1 && errno == EINVAL);
	CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_RECV, NULL),
		   _ret == -1 && errno == EFAULT);
	CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_SEND, NULL),
		   _ret == -1 && errno == EFAULT);
	CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_ID_VALID, NULL),
		   _ret == -1 && errno == EFAULT);
	pid_t child = notifying_child(fd, 555, 0);
	struct seccomp_notif request = receive(fd, child);
	struct seccomp_notif_resp response = { .id = request.id, .flags = 2 };
	CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_SEND, &response),
		   _ret == -1 && errno == EINVAL);
	response.flags = SECCOMP_USER_NOTIF_FLAG_CONTINUE;
	response.val = 1;
	CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_SEND, &response),
		   _ret == -1 && errno == EINVAL);
	response.val = 0;
	response.error = -EPERM;
	CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_SEND, &response),
		   _ret == -1 && errno == EINVAL);
	response.error = 0;
	response.flags = 0;
	response.id++;
	CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_SEND, &response),
		   _ret == -1 && errno == ENOENT);
	response.id = request.id;
	response.val = 555;
	CHECK(ioctl(fd, SECCOMP_IOCTL_NOTIF_ID_VALID, &request.id));
	CHECK(ioctl(fd, SECCOMP_IOCTL_NOTIF_SEND, &response));
	reap(child);
	CHECK(close(fd));
}

static void receive_fault_requeues_notification(void)
{
	int fd = listener();
	pid_t child = notifying_child(fd, 777, 0);
	struct pollfd pfd = {
		.fd = fd, .events = POLLIN | POLLRDNORM | POLLOUT | POLLWRNORM
	};
	CHECK_WITH(poll(&pfd, 1, 5000),
		   _ret == 1 && pfd.revents == (POLLIN | POLLRDNORM));
	size_t size = sysconf(_SC_PAGESIZE);
	void *page =
		mmap(NULL, size, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	CHECK_WITH(page != MAP_FAILED, _ret);
	CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_RECV, page),
		   _ret == -1 && errno == EFAULT);
	CHECK_WITH(poll(&pfd, 1, 0),
		   _ret == 1 && pfd.revents == (POLLIN | POLLRDNORM));
	struct seccomp_notif request = receive(fd, child);
	CHECK_WITH(poll(&pfd, 1, 0),
		   _ret == 1 && pfd.revents == (POLLOUT | POLLWRNORM));
	struct seccomp_notif_resp response = { .id = request.id, .val = 777 };
	CHECK(ioctl(fd, SECCOMP_IOCTL_NOTIF_SEND, &response));
	reap(child);
	CHECK_WITH(poll(&pfd, 1, 0), _ret == 0);
	CHECK(munmap(page, size));
	CHECK(close(fd));
}

static void receive_validates_before_later_page_faults(void)
{
	int fd = listener();
	size_t size = sysconf(_SC_PAGESIZE);
	unsigned char *pages = mmap(NULL, size * 2, PROT_READ | PROT_WRITE,
				    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	CHECK_WITH(pages != MAP_FAILED, _ret);
	CHECK(mprotect(pages + size, size, PROT_NONE));
	/* Also cover an unaligned first byte next to an inaccessible page. */
	for (size_t offset = 1; offset <= sizeof(struct seccomp_notif) / 2;
	     ++offset) {
		unsigned char *buffer = pages + size - offset;
		*buffer = 1;
		CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_RECV, buffer),
			   _ret == -1 && errno == EINVAL);
		*buffer = 0;
		CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_RECV, buffer),
			   _ret == -1 && errno == EFAULT);
	}
	CHECK(munmap(pages, size * 2));
	CHECK(close(fd));
}

static void closing_listener_releases_pending_call(void)
{
	int fd = listener();
	pid_t child = notifying_child(fd, -1, ENOSYS);
	receive(fd, child);
	CHECK(close(fd));
	reap(child);
}

static void signal_handler(int signal)
{
	(void)signal;
}

static void signal_cancels_pending_call(void)
{
	struct sigaction action = { .sa_handler = signal_handler };
	CHECK(sigemptyset(&action.sa_mask));
	CHECK(sigaction(SIGUSR1, &action, NULL));
	int fd = listener();
	pid_t child = notifying_child(fd, -1, EINTR);
	struct seccomp_notif request = receive(fd, child);
	CHECK(kill(child, SIGUSR1));
	reap(child);
	CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_ID_VALID, &request.id),
		   _ret == -1 && errno == ENOENT);
	struct pollfd pfd = { .fd = fd, .events = POLLIN | POLLOUT };
	CHECK_WITH(poll(&pfd, 1, 0), _ret == 0);
	CHECK(close(fd));
}

static void killed_task_invalidates_notification(void)
{
	int fd = listener();
	pid_t child = notifying_child(fd, 0, 0);
	struct seccomp_notif request = receive(fd, child);
	CHECK(kill(child, SIGKILL));
	int status;
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFSIGNALED(status) &&
			   WTERMSIG(status) == SIGKILL);
	CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_ID_VALID, &request.id),
		   _ret == -1 && errno == ENOENT);
	struct seccomp_notif_resp response = { .id = request.id };
	CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_SEND, &response),
		   _ret == -1 && errno == ENOENT);
	CHECK(close(fd));
}

static void notification_response_is_logged(void)
{
	int fd = listener_flags(SECCOMP_FILTER_FLAG_LOG);
	pid_t child = notifying_child_logged(fd, 888, 0, 1);
	struct seccomp_notif request = receive(fd, child);
	struct seccomp_notif_resp response = { .id = request.id, .val = 888 };
	CHECK(ioctl(fd, SECCOMP_IOCTL_NOTIF_SEND, &response));
	reap(child);
	CHECK(close(fd));
}

static void notification_continue_is_silent(void)
{
	int fd = listener_flags(SECCOMP_FILTER_FLAG_LOG);
	pid_t child = notifying_child_logged(fd, getpid(), 0, 0);
	struct seccomp_notif request = receive(fd, child);
	struct seccomp_notif_resp response = {
		.id = request.id,
		.flags = SECCOMP_USER_NOTIF_FLAG_CONTINUE,
	};
	CHECK(ioctl(fd, SECCOMP_IOCTL_NOTIF_SEND, &response));
	reap(child);
	CHECK(close(fd));
}

FN_TEST(notification)
{
	struct log_capture capture = capture_log_output();
	alarm(60);
	uint32_t action = SECCOMP_RET_USER_NOTIF;
	TEST_SUCC(syscall(SYS_seccomp, SECCOMP_GET_ACTION_AVAIL, 0, &action));
	TEST_RES(run_child(no_listener_returns_enosys, 0), _ret == 0);
	TEST_RES(run_child(closed_listener_returns_enosys, 0), _ret == 0);
	TEST_RES(run_child(responses_and_ids, 0), _ret == 0);
	TEST_RES(run_child(newest_filter_owns_notification, 0), _ret == 0);
	TEST_RES(run_child(errno_precedes_notification, 0), _ret == 0);
	TEST_RES(run_child(invalid_requests_preserve_notification, 0),
		 _ret == 0);
	TEST_RES(run_child(receive_fault_requeues_notification, 0), _ret == 0);
	TEST_RES(run_child(closing_listener_releases_pending_call, 0),
		 _ret == 0);
	TEST_RES(run_child(signal_cancels_pending_call, 0), _ret == 0);
	TEST_RES(run_child(killed_task_invalidates_notification, 0), _ret == 0);
	TEST_RES(run_child(receive_validates_before_later_page_faults, 0),
		 _ret == 0);
	TEST_RES(run_child(notification_response_is_logged, 0), _ret == 0);
	TEST_RES(run_child(notification_continue_is_silent, 0), _ret == 0);
	alarm(0);
	flush_log_output(capture);
}
END_TEST()
