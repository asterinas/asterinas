// SPDX-License-Identifier: MPL-2.0

#include "notification_common.h"
#include "seccomp_fd_pass.h"
#include <poll.h>
#include <sched.h>
#include <sys/mman.h>
#include <time.h>

static int signal_report[2];

static void report_signal(int signal)
{
	(void)signal;
	int saved_errno = errno;
	if (write(signal_report[1], "s", 1) != 1)
		_exit(1);
	errno = saved_errno;
}

static void setup_signal_handler(int flags)
{
	CHECK(pipe(signal_report));
	struct sigaction action = { .sa_handler = report_signal,
				    .sa_flags = flags };
	CHECK(sigemptyset(&action.sa_mask));
	CHECK(sigaction(SIGUSR1, &action, NULL));
}

static void collect_signal(void)
{
	struct pollfd pfd = { .fd = signal_report[0], .events = POLLIN };
	CHECK_WITH(poll(&pfd, 1, 5000), _ret == 1 && pfd.revents == POLLIN);
	char byte;
	CHECK_WITH(read(signal_report[0], &byte, 1), _ret == 1 && byte == 's');
}

static void close_signal_report(void)
{
	CHECK(close(signal_report[0]));
	CHECK(close(signal_report[1]));
}

static void wait_state(pid_t pid, char expected)
{
	struct timespec start, now;
	CHECK(clock_gettime(CLOCK_MONOTONIC, &start));
	char path[64];
	snprintf(path, sizeof(path), "/proc/%d/stat", pid);
	for (;;) {
		int fd = CHECK(open(path, O_RDONLY));
		char buffer[1024];
		ssize_t len = CHECK(read(fd, buffer, sizeof(buffer) - 1));
		CHECK_WITH(len, _ret > 0);
		buffer[len] = '\0';
		CHECK(close(fd));
		char *end = CHECK_WITH(strrchr(buffer, ')'), _ret != NULL);
		if (end[2] == expected)
			return;
		CHECK_WITH(end[2], _ret != 'Z');
		CHECK(clock_gettime(CLOCK_MONOTONIC, &now));
		CHECK_WITH(now.tv_sec - start.tv_sec, _ret < 10);
		CHECK(sched_yield());
	}
}

static void wait_readable(int fd)
{
	struct pollfd pfd = { .fd = fd, .events = POLLIN };
	CHECK_WITH(poll(&pfd, 1, 5000), _ret == 1 && pfd.revents == POLLIN);
}

static void reply(int fd, uint64_t id, long value)
{
	struct seccomp_notif_resp response = { .id = id, .val = value };
	CHECK(ioctl(fd, SECCOMP_IOCTL_NOTIF_SEND, &response));
}

static void invalid_id(int fd, uint64_t id)
{
	CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_ID_VALID, &id),
		   _ret == -1 && errno == ENOENT);
}

static void killable_flag_validation(void)
{
	no_new_privs();
	CHECK_WITH(syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER,
			   SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV, NULL),
		   _ret == -1 && errno == EINVAL);
	CHECK_WITH(notification_filter(SECCOMP_RET_ALLOW,
				       SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV),
		   _ret == -1 && errno == EINVAL);
	CHECK_WITH(
		notification_filter(SECCOMP_RET_ALLOW,
				    SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV |
					    SECCOMP_FILTER_FLAG_TSYNC |
					    SECCOMP_FILTER_FLAG_NEW_LISTENER),
		_ret == -1 && errno == EINVAL);
	unsigned int flags[] = {
		SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV,
		SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV |
			SECCOMP_FILTER_FLAG_LOG,
		SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV |
			SECCOMP_FILTER_FLAG_TSYNC |
			SECCOMP_FILTER_FLAG_TSYNC_ESRCH,
	};
	for (size_t i = 0; i < sizeof(flags) / sizeof(flags[0]); ++i) {
		int fd = CHECK(notification_filter(
			SECCOMP_RET_ALLOW,
			flags[i] | SECCOMP_FILTER_FLAG_NEW_LISTENER));
		CHECK(close(fd));
	}
}

static void signal_before_receive_cancels(void)
{
	setup_signal_handler(0);
	int fd = listener_flags(SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV);
	pid_t child = notifying_child(fd, -1, EINTR);
	wait_readable(fd);
	wait_state(child, 'S');
	CHECK(kill(child, SIGUSR1));
	collect_signal();
	reap(child);
	struct seccomp_notif request = { 0 };
	CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_RECV, &request),
		   _ret == -1 && errno == ENOENT);
	CHECK(close(fd));
	close_signal_report();
}

static void signal_after_receive_waits_for_reply(void)
{
	setup_signal_handler(0);
	int fd = listener_flags(SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV);
	pid_t child = notifying_child(fd, 777, 0);
	struct seccomp_notif request = receive(fd, child);
	CHECK(kill(child, SIGUSR1));
	wait_state(child, 'D');
	struct pollfd report = { .fd = signal_report[0], .events = POLLIN };
	CHECK_WITH(poll(&report, 1, 0), _ret == 0);
	CHECK(ioctl(fd, SECCOMP_IOCTL_NOTIF_ID_VALID, &request.id));
	reply(fd, request.id, 777);
	reap(child);
	collect_signal();
	invalid_id(fd, request.id);
	CHECK(close(fd));
	close_signal_report();
}

static void check_fatal_signal(int fd, int signo)
{
	pid_t child = notifying_child(fd, 0, 0);
	struct seccomp_notif request = receive(fd, child);
	CHECK(kill(child, SIGUSR1));
	wait_state(child, 'D');
	CHECK(kill(child, signo));
	int status;
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFSIGNALED(status) &&
			   WTERMSIG(status) == signo);
	invalid_id(fd, request.id);
}

static void fatal_signals_end_killable_wait(void)
{
	setup_signal_handler(0);
	int fd = listener_flags(SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV);
	check_fatal_signal(fd, SIGKILL);
	check_fatal_signal(fd, SIGTERM);
	CHECK(close(fd));
	close_signal_report();
}

static void receive_fault_restores_interruptibility(void)
{
	setup_signal_handler(0);
	int fd = listener_flags(SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV);
	pid_t child = notifying_child(fd, -1, EINTR);
	wait_readable(fd);
	size_t size = sysconf(_SC_PAGESIZE);
	void *page =
		mmap(NULL, size, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	CHECK_WITH(page != MAP_FAILED, _ret);
	CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_RECV, page),
		   _ret == -1 && errno == EFAULT);
	wait_readable(fd);
	CHECK(kill(child, SIGUSR1));
	collect_signal();
	reap(child);
	struct seccomp_notif request = { 0 };
	CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_RECV, &request),
		   _ret == -1 && errno == ENOENT);
	CHECK(munmap(page, size));
	CHECK(close(fd));
	close_signal_report();
}

static void restarting_call_gets_new_id(void)
{
	setup_signal_handler(SA_RESTART);
	int fd = listener();
	pid_t child = notifying_child(fd, 888, 0);
	struct seccomp_notif first = receive(fd, child);
	CHECK(kill(child, SIGUSR1));
	collect_signal();
	struct seccomp_notif second = receive(fd, child);
	CHECK_WITH(second.id != first.id, _ret);
	invalid_id(fd, first.id);
	reply(fd, second.id, 888);
	reap(child);
	invalid_id(fd, second.id);
	CHECK(close(fd));
	close_signal_report();
}

static pthread_barrier_t barrier;

struct receiver {
	int fd;
	struct seccomp_notif request;
};

static void *receive_and_reply(void *arg)
{
	struct receiver *receiver = arg;
	pthread_barrier_wait(&barrier);
	CHECK(ioctl(receiver->fd, SECCOMP_IOCTL_NOTIF_RECV,
		    &receiver->request));
	reply(receiver->fd, receiver->request.id, 777);
	return NULL;
}

static void competing_receivers_get_distinct_requests(void)
{
	int fd = listener();
	for (int round = 0; round < 8; ++round) {
		CHECK(pthread_barrier_init(&barrier, NULL, 3));
		struct receiver receivers[2] = { { .fd = fd }, { .fd = fd } };
		pthread_t threads[2];
		for (int i = 0; i < 2; ++i)
			CHECK(pthread_create(&threads[i], NULL,
					     receive_and_reply, &receivers[i]));
		pthread_barrier_wait(&barrier);
		pid_t first = notifying_child(fd, 777, 0);
		pid_t second = notifying_child(fd, 777, 0);
		for (int i = 0; i < 2; ++i)
			CHECK(pthread_join(threads[i], NULL));
		CHECK_WITH(receivers[0].request.id != receivers[1].request.id,
			   _ret);
		CHECK_WITH((receivers[0].request.pid == first &&
			    receivers[1].request.pid == second) ||
				   (receivers[0].request.pid == second &&
				    receivers[1].request.pid == first),
			   _ret);
		reap(first);
		reap(second);
		CHECK(pthread_barrier_destroy(&barrier));
	}
	CHECK(close(fd));
}

struct responder {
	int fd;
	uint64_t id;
	int result;
	int error;
};

static void *send_response(void *arg)
{
	struct responder *responder = arg;
	struct seccomp_notif_resp response = { .id = responder->id,
					       .val = 777 };
	pthread_barrier_wait(&barrier);
	responder->result =
		ioctl(responder->fd, SECCOMP_IOCTL_NOTIF_SEND, &response);
	responder->error = errno;
	return NULL;
}

static void competing_replies_complete_once(void)
{
	int fd = listener();
	for (int round = 0; round < 8; ++round) {
		pid_t child = notifying_child(fd, 777, 0);
		struct seccomp_notif request = receive(fd, child);
		CHECK(pthread_barrier_init(&barrier, NULL, 3));
		struct responder responders[2] = {
			{ .fd = fd, .id = request.id },
			{ .fd = fd, .id = request.id },
		};
		pthread_t threads[2];
		for (int i = 0; i < 2; ++i)
			CHECK(pthread_create(&threads[i], NULL, send_response,
					     &responders[i]));
		pthread_barrier_wait(&barrier);
		for (int i = 0; i < 2; ++i)
			CHECK(pthread_join(threads[i], NULL));
		CHECK_WITH((responders[0].result == 0) +
				   (responders[1].result == 0),
			   _ret == 1);
		struct responder *loser =
			responders[0].result ? &responders[0] : &responders[1];
		CHECK_WITH(loser->result == -1 && (loser->error == ENOENT ||
						   loser->error == EINPROGRESS),
			   _ret);
		reap(child);
		invalid_id(fd, request.id);
		CHECK(pthread_barrier_destroy(&barrier));
	}
	CHECK(close(fd));
}

static void *interrupt_notifier(void *arg)
{
	pid_t child = *(pid_t *)arg;
	pthread_barrier_wait(&barrier);
	CHECK(kill(child, SIGUSR1));
	return NULL;
}

struct racing_receiver {
	int fd;
	pid_t child;
	int received;
};

static void *receive_or_cancel(void *arg)
{
	struct racing_receiver *receiver = arg;
	struct seccomp_notif request = { 0 };
	pthread_barrier_wait(&barrier);
	int result = ioctl(receiver->fd, SECCOMP_IOCTL_NOTIF_RECV, &request);
	int error = errno;
	CHECK_WITH(result, _ret == 0 || (_ret == -1 && error == ENOENT));
	receiver->received = result == 0;
	if (receiver->received) {
		CHECK_WITH(request.pid, _ret == receiver->child);
		reply(receiver->fd, request.id, 777);
	}
	return NULL;
}

static void receive_races_with_signal(void)
{
	setup_signal_handler(0);
	int fd = listener_flags(SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV);
	for (int round = 0; round < 32; ++round) {
		int result_pipe[2], gate[2];
		CHECK(pipe(result_pipe));
		CHECK(pipe(gate));
		pid_t child = CHECK(fork());
		if (!child) {
			alarm(10);
			CHECK(close(fd));
			CHECK(close(result_pipe[0]));
			CHECK(close(gate[1]));
			errno = 0;
			long result[2];
			result[0] = syscall(SYS_getppid);
			result[1] = errno;
			CHECK_WITH(write(result_pipe[1], result,
					 sizeof(result)),
				   _ret == sizeof(result));
			/* Keep the PID alive until the signal sender has finished. */
			char byte;
			ssize_t count;
			do {
				count = read(gate[0], &byte, 1);
			} while (count == -1 && errno == EINTR);
			CHECK_WITH(count, _ret == 1);
			_exit(0);
		}
		CHECK(close(result_pipe[1]));
		CHECK(close(gate[0]));
		wait_readable(fd);
		wait_state(child, 'S');
		CHECK(pthread_barrier_init(&barrier, NULL, 3));
		struct racing_receiver receiver = { .fd = fd, .child = child };
		pthread_t reader, interrupter;
		CHECK(pthread_create(&reader, NULL, receive_or_cancel,
				     &receiver));
		CHECK(pthread_create(&interrupter, NULL, interrupt_notifier,
				     &child));
		pthread_barrier_wait(&barrier);
		CHECK(pthread_join(reader, NULL));
		CHECK(pthread_join(interrupter, NULL));
		long result[2];
		CHECK_WITH(read(result_pipe[0], result, sizeof(result)),
			   _ret == sizeof(result));
		CHECK_WITH(result[0],
			   receiver.received ?
				   (_ret == 777 && result[1] == 0) :
				   (_ret == -1 && result[1] == EINTR));
		collect_signal();
		CHECK_WITH(write(gate[1], "x", 1), _ret == 1);
		reap(child);
		CHECK(close(result_pipe[0]));
		CHECK(close(gate[1]));
		CHECK(pthread_barrier_destroy(&barrier));
	}
	CHECK(close(fd));
	close_signal_report();
}

static void reply_races_with_signal(void)
{
	setup_signal_handler(0);
	int fd = listener_flags(SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV);
	for (int round = 0; round < 32; ++round) {
		pid_t child = notifying_child(fd, 777, 0);
		struct seccomp_notif request = receive(fd, child);
		struct responder responder = { .fd = fd, .id = request.id };
		CHECK(pthread_barrier_init(&barrier, NULL, 3));
		pthread_t sender, interrupter;
		CHECK(pthread_create(&sender, NULL, send_response, &responder));
		CHECK(pthread_create(&interrupter, NULL, interrupt_notifier,
				     &child));
		pthread_barrier_wait(&barrier);
		CHECK(pthread_join(sender, NULL));
		CHECK(pthread_join(interrupter, NULL));
		CHECK_WITH(responder.result, _ret == 0);
		reap(child);
		invalid_id(fd, request.id);
		CHECK(pthread_barrier_destroy(&barrier));
	}
	CHECK(close(fd));
	close_signal_report();
}

static void last_duplicate_close_releases_queued_call(void)
{
	int fd = listener();
	/* The child must not inherit the duplicate that keeps the listener open. */
	pid_t child = notifying_child(fd, -1, ENOSYS);
	int copy = CHECK(dup(fd));
	wait_readable(fd);
	CHECK(close(fd));
	wait_readable(copy);
	CHECK(close(copy));
	reap(child);
}

static void accepted_reply_survives_listener_close(void)
{
	int fd = listener();
	pid_t child = notifying_child(fd, 999, 0);
	struct seccomp_notif request = receive(fd, child);
	reply(fd, request.id, 999);
	CHECK(close(fd));
	reap(child);
}

static void nonblocking_receive_still_waits(void)
{
	setup_signal_handler(0);
	int fd = listener();
	CHECK(fcntl(fd, F_SETFL, O_NONBLOCK));
	pid_t receiver = CHECK(fork());
	if (!receiver) {
		alarm(10);
		struct seccomp_notif request = { 0 };
		CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_RECV, &request),
			   _ret == -1 && errno == EINTR);
		_exit(0);
	}
	wait_state(receiver, 'S');
	CHECK(kill(receiver, SIGUSR1));
	collect_signal();
	reap(receiver);
	CHECK(close(fd));
	close_signal_report();
}

static void last_filter_exit_wakes_receiver(void)
{
	int sockets[2], gate[2];
	CHECK(socketpair(AF_UNIX, SOCK_STREAM, 0, sockets));
	CHECK(pipe(gate));
	pid_t filtered = CHECK(fork());
	if (!filtered) {
		alarm(10);
		CHECK(close(sockets[0]));
		CHECK(close(gate[1]));
		int fd = listener_flags(SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV);
		send_fd(sockets[1], fd);
		CHECK(close(fd));
		CHECK(close(sockets[1]));
		char byte;
		CHECK_WITH(read(gate[0], &byte, 1), _ret == 1);
		_exit(0);
	}
	CHECK(close(sockets[1]));
	CHECK(close(gate[0]));
	int fd = receive_fd(sockets[0]);
	pid_t receiver = CHECK(fork());
	if (!receiver) {
		alarm(10);
		CHECK(close(gate[1]));
		CHECK(close(sockets[0]));
		struct seccomp_notif request = { 0 };
		CHECK_WITH(ioctl(fd, SECCOMP_IOCTL_NOTIF_RECV, &request),
			   _ret == -1 && errno == ENOENT);
		struct pollfd pfd = { .fd = fd, .events = POLLIN | POLLOUT };
		CHECK_WITH(poll(&pfd, 1, 0),
			   _ret == 1 && pfd.revents == POLLHUP);
		CHECK(close(fd));
		_exit(0);
	}
	wait_state(receiver, 'S');
	CHECK_WITH(write(gate[1], "x", 1), _ret == 1);
	reap(filtered);
	reap(receiver);
	CHECK(close(fd));
	CHECK(close(gate[1]));
	CHECK(close(sockets[0]));
}

static int run_lifecycle_child(void (*body)(void))
{
	fflush(NULL);
	pid_t child = CHECK(fork());
	if (!child) {
		alarm(30);
		body();
		_exit(0);
	}
	int status;
	CHECK_WITH(waitpid(child, &status, 0), _ret == child);
	return !(WIFEXITED(status) && WEXITSTATUS(status) == 0);
}

FN_TEST(notification_lifecycle)
{
	alarm(90);
	TEST_RES(run_lifecycle_child(killable_flag_validation), _ret == 0);
	TEST_RES(run_lifecycle_child(signal_before_receive_cancels), _ret == 0);
	TEST_RES(run_lifecycle_child(signal_after_receive_waits_for_reply),
		 _ret == 0);
	TEST_RES(run_lifecycle_child(fatal_signals_end_killable_wait),
		 _ret == 0);
	TEST_RES(run_lifecycle_child(receive_fault_restores_interruptibility),
		 _ret == 0);
	TEST_RES(run_lifecycle_child(restarting_call_gets_new_id), _ret == 0);
	TEST_RES(run_lifecycle_child(competing_receivers_get_distinct_requests),
		 _ret == 0);
	TEST_RES(run_lifecycle_child(competing_replies_complete_once),
		 _ret == 0);
	TEST_RES(run_lifecycle_child(last_duplicate_close_releases_queued_call),
		 _ret == 0);
	TEST_RES(run_lifecycle_child(accepted_reply_survives_listener_close),
		 _ret == 0);
	TEST_RES(run_lifecycle_child(nonblocking_receive_still_waits),
		 _ret == 0);
	TEST_RES(run_lifecycle_child(last_filter_exit_wakes_receiver),
		 _ret == 0);
	TEST_RES(run_lifecycle_child(reply_races_with_signal), _ret == 0);
	TEST_RES(run_lifecycle_child(receive_races_with_signal), _ret == 0);
	alarm(0);
}
END_TEST()
