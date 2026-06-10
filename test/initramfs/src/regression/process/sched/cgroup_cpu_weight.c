// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <sched.h>
#include <signal.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#define CGROUP_ROOT "/sys/fs/cgroup"
#define MAX_WORKERS 4
#define FAIR_WINDOW_SECONDS 8
#define STARVATION_WINDOW_SECONDS 10
#define MIN_FAIR_TOTAL_USEC 4000000ULL
#define MIN_STARVATION_TOTAL_USEC 5000000ULL

struct worker {
	pid_t pid;
	int start_fd;
};

static char group_a[PATH_MAX];
static char group_b[PATH_MAX];
static char guard_group[PATH_MAX];
static struct worker workers[MAX_WORKERS];
static size_t worker_count;

static void die(const char *message)
{
	perror(message);
	exit(EXIT_FAILURE);
}

static void fail(const char *message)
{
	fprintf(stderr, "FAIL: %s\n", message);
	exit(EXIT_FAILURE);
}

static void checked_snprintf(char *buffer, size_t size, const char *format, ...)
{
	va_list args;
	int count;

	va_start(args, format);
	count = vsnprintf(buffer, size, format, args);
	va_end(args);

	if (count < 0 || (size_t)count >= size)
		fail("formatted string is too long");
}

static void write_text_file(const char *path, const char *text)
{
	int fd = open(path, O_WRONLY);
	size_t len = strlen(text);
	ssize_t count;

	if (fd < 0)
		die(path);

	count = write(fd, text, len);
	if (count != (ssize_t)len) {
		if (count >= 0)
			errno = EIO;
		close(fd);
		die(path);
	}

	if (close(fd) < 0)
		die(path);
}

static void expect_write_errno(const char *path, const char *text,
			       int expected_errno)
{
	int fd = open(path, O_WRONLY);
	ssize_t count;

	if (fd < 0)
		die(path);

	errno = 0;
	count = write(fd, text, strlen(text));
	if (count >= 0 || errno != expected_errno) {
		close(fd);
		fprintf(stderr,
			"FAIL: writing \"%s\" to %s got count=%zd errno=%d, expected errno=%d\n",
			text, path, count, errno, expected_errno);
		exit(EXIT_FAILURE);
	}

	if (close(fd) < 0)
		die(path);
}

static ssize_t read_text_file(const char *path, char *buffer, size_t size)
{
	int fd = open(path, O_RDONLY);
	ssize_t count;

	if (fd < 0)
		die(path);

	count = read(fd, buffer, size - 1);
	if (count < 0) {
		close(fd);
		die(path);
	}
	buffer[count] = '\0';

	if (close(fd) < 0)
		die(path);

	return count;
}

static int has_token(const char *text, const char *token)
{
	size_t token_len = strlen(token);
	const char *cursor = text;

	while ((cursor = strstr(cursor, token)) != NULL) {
		int before_ok = cursor == text || cursor[-1] == ' ' ||
				cursor[-1] == '\n' || cursor[-1] == '\t';
		int after_ok =
			cursor[token_len] == '\0' || cursor[token_len] == ' ' ||
			cursor[token_len] == '\n' || cursor[token_len] == '\t';

		if (before_ok && after_ok)
			return 1;
		cursor += token_len;
	}

	return 0;
}

static void enable_cpu_controller(void)
{
	char content[256];
	char path[PATH_MAX];

	checked_snprintf(path, sizeof(path), "%s/cgroup.subtree_control",
			 CGROUP_ROOT);
	read_text_file(path, content, sizeof(content));
	if (!has_token(content, "cpu"))
		write_text_file(path, "+cpu\n");
}

static void disable_cpu_controller(void)
{
	char content[256];
	char path[PATH_MAX];

	checked_snprintf(path, sizeof(path), "%s/cgroup.subtree_control",
			 CGROUP_ROOT);
	read_text_file(path, content, sizeof(content));
	if (has_token(content, "cpu"))
		write_text_file(path, "-cpu\n");
}

static void set_weight(const char *group, unsigned int weight)
{
	char path[PATH_MAX];
	char text[32];

	checked_snprintf(path, sizeof(path), "%s/cpu.weight", group);
	snprintf(text, sizeof(text), "%u\n", weight);
	write_text_file(path, text);
}

static unsigned int read_weight(const char *group)
{
	char path[PATH_MAX];
	char content[64];
	char *end;
	unsigned long value;

	checked_snprintf(path, sizeof(path), "%s/cpu.weight", group);
	read_text_file(path, content, sizeof(content));
	errno = 0;
	value = strtoul(content, &end, 10);
	if (errno != 0 || end == content || value > UINT_MAX)
		die(path);

	return (unsigned int)value;
}

static void move_pid_to_cgroup(const char *group, pid_t pid)
{
	char path[PATH_MAX];
	char text[32];

	checked_snprintf(path, sizeof(path), "%s/cgroup.procs", group);
	snprintf(text, sizeof(text), "%d", pid);
	write_text_file(path, text);
}

static unsigned long long read_usage_usec(const char *group)
{
	char path[PATH_MAX];
	char content[512];
	char *usage;

	checked_snprintf(path, sizeof(path), "%s/cpu.stat", group);
	read_text_file(path, content, sizeof(content));

	usage = strstr(content, "usage_usec ");
	if (usage == NULL)
		fail("cpu.stat does not contain usage_usec");

	errno = 0;
	usage += strlen("usage_usec ");
	unsigned long long value = strtoull(usage, NULL, 10);
	if (errno != 0)
		die(path);

	return value;
}

static int pin_current_to_cpu0(void)
{
	cpu_set_t mask;

	CPU_ZERO(&mask);
	CPU_SET(0, &mask);
	return sched_setaffinity(0, sizeof(mask), &mask);
}

static void worker_loop(int start_fd)
{
	char byte;
	volatile unsigned long counter = 0;

	if (pin_current_to_cpu0() < 0)
		_exit(EXIT_FAILURE);
	if (kill(getpid(), SIGSTOP) < 0)
		_exit(EXIT_FAILURE);
	if (read(start_fd, &byte, 1) != 1)
		_exit(EXIT_SUCCESS);

	for (;;) {
		counter++;
		if ((counter & 0xfffffUL) == 0)
			asm volatile("" : : "r"(counter) : "memory");
	}
}

static void wait_for_worker_stop(pid_t pid)
{
	int status;

	for (;;) {
		if (waitpid(pid, &status, WUNTRACED) < 0) {
			if (errno == EINTR)
				continue;
			die("waitpid");
		}

		if (!WIFSTOPPED(status))
			fail("worker exited before it stopped");
		return;
	}
}

static void wait_for_worker_exit(pid_t pid)
{
	for (;;) {
		if (waitpid(pid, NULL, 0) >= 0)
			return;
		if (errno == EINTR)
			continue;
		if (errno == ECHILD)
			return;
		die("waitpid");
	}
}

static void spawn_worker(const char *group)
{
	int pipe_fds[2];
	pid_t pid;
	struct worker worker;

	if (worker_count >= MAX_WORKERS)
		fail("too many workers");

	if (pipe(pipe_fds) < 0)
		die("pipe");

	pid = fork();
	if (pid < 0)
		die("fork");
	if (pid == 0) {
		close(pipe_fds[1]);
		worker_loop(pipe_fds[0]);
		_exit(EXIT_SUCCESS);
	}

	close(pipe_fds[0]);
	worker.pid = pid;
	worker.start_fd = pipe_fds[1];
	workers[worker_count++] = worker;

	wait_for_worker_stop(pid);
	move_pid_to_cgroup(group, pid);
}

static void start_workers(void)
{
	for (size_t index = 0; index < worker_count; index++) {
		if (write(workers[index].start_fd, "x", 1) != 1)
			die("start worker");
		close(workers[index].start_fd);
		workers[index].start_fd = -1;
	}

	for (size_t index = 0; index < worker_count; index++) {
		if (kill(workers[index].pid, SIGCONT) < 0)
			die("continue worker");
	}
}

static void stop_workers(void)
{
	for (size_t index = 0; index < worker_count; index++) {
		if (workers[index].start_fd >= 0) {
			close(workers[index].start_fd);
			workers[index].start_fd = -1;
		}
		if (workers[index].pid > 0)
			kill(workers[index].pid, SIGKILL);
	}

	for (size_t index = 0; index < worker_count; index++) {
		if (workers[index].pid > 0)
			wait_for_worker_exit(workers[index].pid);
		workers[index].pid = 0;
	}

	worker_count = 0;
}

static void cleanup(void)
{
	stop_workers();
	if (guard_group[0] != '\0')
		rmdir(guard_group);
	if (group_b[0] != '\0')
		rmdir(group_b);
	if (group_a[0] != '\0')
		rmdir(group_a);
}

static void create_test_cgroups(void)
{
	char suffix[64];

	checked_snprintf(suffix, sizeof(suffix), "cpuw-%d", getpid());
	checked_snprintf(group_a, sizeof(group_a), "%s/%s-a", CGROUP_ROOT,
			 suffix);
	checked_snprintf(group_b, sizeof(group_b), "%s/%s-b", CGROUP_ROOT,
			 suffix);

	if (mkdir(group_a, 0755) < 0)
		die(group_a);
	if (mkdir(group_b, 0755) < 0)
		die(group_b);
}

struct usage_sample {
	unsigned long long delta_a;
	unsigned long long delta_b;
};

static struct usage_sample
measure_existing_workers_current(int workers_a, int workers_b,
				 unsigned int window_seconds)
{
	unsigned long long before_a;
	unsigned long long before_b;
	unsigned long long delta_a;
	unsigned long long delta_b;

	before_a = read_usage_usec(group_a);
	before_b = read_usage_usec(group_b);

	start_workers();
	sleep(window_seconds);

	delta_a = read_usage_usec(group_a) - before_a;
	delta_b = read_usage_usec(group_b) - before_b;

	stop_workers();

	fprintf(stderr, "current weights workers=%d:%d usage=%llu:%llu\n",
		workers_a, workers_b, delta_a, delta_b);

	return (struct usage_sample){
		.delta_a = delta_a,
		.delta_b = delta_b,
	};
}

static struct usage_sample measure_usage_current(int workers_a, int workers_b,
						 unsigned int window_seconds)
{
	for (int index = 0; index < workers_a; index++)
		spawn_worker(group_a);
	for (int index = 0; index < workers_b; index++)
		spawn_worker(group_b);

	return measure_existing_workers_current(workers_a, workers_b,
						window_seconds);
}

static struct usage_sample measure_usage(unsigned int weight_a,
					 unsigned int weight_b, int workers_a,
					 int workers_b,
					 unsigned int window_seconds)
{
	set_weight(group_a, weight_a);
	set_weight(group_b, weight_b);

	return measure_usage_current(workers_a, workers_b, window_seconds);
}

static unsigned int share_basis_points(const struct usage_sample *sample,
				       unsigned long long min_total)
{
	unsigned long long total = sample->delta_a + sample->delta_b;

	if (total < min_total)
		fail("cpu.stat usage delta is too small");

	return (unsigned int)(sample->delta_a * 10000 / total);
}

static void expect_share_range(const char *name, unsigned int share_bp,
			       unsigned int min_bp, unsigned int max_bp)
{
	if (share_bp < min_bp || share_bp > max_bp) {
		fprintf(stderr,
			"FAIL: %s share_a=%u.%02u%%, expected %u.%02u..%u.%02u%%\n",
			name, share_bp / 100, share_bp % 100, min_bp / 100,
			min_bp % 100, max_bp / 100, max_bp % 100);
		exit(EXIT_FAILURE);
	}

	fprintf(stderr, "%s share_a=%u.%02u%%\n", name, share_bp / 100,
		share_bp % 100);
}

static void test_invalid_weights(void)
{
	char path[PATH_MAX];

	set_weight(group_a, 100);
	checked_snprintf(path, sizeof(path), "%s/cpu.weight", group_a);

	expect_write_errno(path, "0\n", EINVAL);
	if (read_weight(group_a) != 100)
		fail("invalid zero weight changed cpu.weight");

	expect_write_errno(path, "10001\n", EINVAL);
	if (read_weight(group_a) != 100)
		fail("invalid large weight changed cpu.weight");
}

static void test_fair_share(const char *name, unsigned int weight_a,
			    unsigned int weight_b, int workers_a, int workers_b,
			    unsigned int min_bp, unsigned int max_bp)
{
	struct usage_sample sample;
	unsigned int share_bp = 0;

	sample = measure_usage(weight_a, weight_b, workers_a, workers_b,
			       FAIR_WINDOW_SECONDS);
	fprintf(stderr, "weights=%u:%u ", weight_a, weight_b);
	share_bp = share_basis_points(&sample, MIN_FAIR_TOTAL_USEC);

	expect_share_range(name, share_bp, min_bp, max_bp);
}

static void test_weight_reset_after_cpu_toggle(void)
{
	struct usage_sample sample;
	unsigned int share_bp;
	char path[PATH_MAX];

	set_weight(group_a, 300);
	set_weight(group_b, 100);

	checked_snprintf(path, sizeof(path), "%s/cgroup.subtree_control",
			 CGROUP_ROOT);
	write_text_file(path, "-cpu\n");
	write_text_file(path, "+cpu\n");

	if (read_weight(group_a) != 100 || read_weight(group_b) != 100)
		fail("cpu.weight did not reset to default after cpu toggle");

	sample = measure_usage_current(1, 1, FAIR_WINDOW_SECONDS);
	share_bp = share_basis_points(&sample, MIN_FAIR_TOTAL_USEC);
	expect_share_range("default weights after cpu toggle", share_bp, 4500,
			   5500);
}

static void test_failed_mixed_disable_is_atomic(void)
{
	char content[256];
	char path[PATH_MAX];
	unsigned long long before_a;
	unsigned long long before_b;
	struct usage_sample sample;
	unsigned int share_bp;

	checked_snprintf(path, sizeof(path), "%s/cgroup.subtree_control",
			 CGROUP_ROOT);
	write_text_file(path, "+cpu +memory\n");

	checked_snprintf(guard_group, sizeof(guard_group), "%s/cpuw-%d-guard",
			 CGROUP_ROOT, getpid());
	if (mkdir(guard_group, 0755) < 0)
		die(guard_group);

	checked_snprintf(path, sizeof(path), "%s/cgroup.subtree_control",
			 guard_group);
	write_text_file(path, "+memory\n");

	set_weight(group_a, 10000);
	set_weight(group_b, 100);
	spawn_worker(group_a);
	spawn_worker(group_b);

	before_a = read_usage_usec(group_a);
	before_b = read_usage_usec(group_b);
	start_workers();

	checked_snprintf(path, sizeof(path), "%s/cgroup.subtree_control",
			 CGROUP_ROOT);
	expect_write_errno(path, "-cpu -memory\n", EBUSY);
	read_text_file(path, content, sizeof(content));
	if (!has_token(content, "cpu") || !has_token(content, "memory"))
		fail("failed mixed disable changed subtree_control");

	sleep(FAIR_WINDOW_SECONDS);

	sample.delta_a = read_usage_usec(group_a) - before_a;
	sample.delta_b = read_usage_usec(group_b) - before_b;
	stop_workers();

	share_bp = share_basis_points(&sample, MIN_FAIR_TOTAL_USEC);
	expect_share_range("failed mixed disable keeps cpu weights", share_bp,
			   8500, 10000);

	checked_snprintf(path, sizeof(path), "%s/cgroup.subtree_control",
			 guard_group);
	write_text_file(path, "-memory\n");
	if (rmdir(guard_group) < 0)
		die(guard_group);
	guard_group[0] = '\0';

	checked_snprintf(path, sizeof(path), "%s/cgroup.subtree_control",
			 CGROUP_ROOT);
	write_text_file(path, "+cpu\n");
}

static void test_existing_members_regroup_after_cpu_enable(void)
{
	struct usage_sample sample;
	unsigned int share_bp;

	spawn_worker(group_a);
	spawn_worker(group_b);

	enable_cpu_controller();
	set_weight(group_a, 300);
	set_weight(group_b, 100);

	sample = measure_existing_workers_current(1, 1, FAIR_WINDOW_SECONDS);
	share_bp = share_basis_points(&sample, MIN_FAIR_TOTAL_USEC);
	expect_share_range("existing members after cpu enable", share_bp, 7000,
			   8000);
}

static void test_extreme_weight_starvation(void)
{
	struct usage_sample sample =
		measure_usage(1, 10000, 1, 1, STARVATION_WINDOW_SECONDS);

	(void)share_basis_points(&sample, MIN_STARVATION_TOTAL_USEC);

	if (sample.delta_a == 0)
		fail("low-weight group received no CPU time");
	if (sample.delta_b <= sample.delta_a)
		fail("high-weight group did not dominate extreme-ratio window");

	fprintf(stderr, "extreme 1:10000 low-weight usage=%llu usec\n",
		sample.delta_a);
}

int main(void)
{
	atexit(cleanup);
	disable_cpu_controller();
	create_test_cgroups();
	test_existing_members_regroup_after_cpu_enable();
	test_invalid_weights();
	test_weight_reset_after_cpu_toggle();
	test_failed_mixed_disable_is_atomic();

	test_fair_share("equal weights", 100, 100, 1, 1, 4500, 5500);
	test_fair_share("100:200 weights", 100, 200, 1, 1, 2833, 3833);
	test_fair_share("equal-weight groups with uneven workers", 100, 100, 2,
			1, 4500, 5500);
	test_fair_share("changed 100:300 weights", 100, 300, 1, 1, 2000, 3000);
	test_extreme_weight_starvation();

	fprintf(stderr, "cgroup cpu.weight regression completed\n");
	return 0;
}
