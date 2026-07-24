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
#define MIGRATION_WINDOW_SECONDS 6
#define MIN_MIGRATION_TOTAL_USEC 3000000ULL

static char base_dir[PATH_MAX];
static char child_dir[PATH_MAX];
static char grandchild_dir[PATH_MAX];
static char load_dir[PATH_MAX];
static char peer_dir[PATH_MAX];

static pid_t stopped_pid = -1;
static pid_t sleeping_pid = -1;
static pid_t load_pid = -1;
static pid_t target_pid = -1;

static void fail(const char *message)
{
	fprintf(stderr, "FAIL: %s\n", message);
	exit(EXIT_FAILURE);
}

static void die(const char *message)
{
	perror(message);
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

static void read_text_file(const char *path, char *buffer, size_t size)
{
	int fd;
	ssize_t count;

	fd = open(path, O_RDONLY);
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
}

static void write_text_file(const char *path, const char *text)
{
	int fd;
	size_t length = strlen(text);
	ssize_t count;

	fd = open(path, O_WRONLY);
	if (fd < 0)
		die(path);

	count = write(fd, text, length);
	if (count != (ssize_t)length) {
		if (count >= 0)
			errno = EIO;
		close(fd);
		die(path);
	}

	if (close(fd) < 0)
		die(path);
}

static void try_write_text_file(const char *path, const char *text)
{
	int fd;
	size_t length = strlen(text);
	ssize_t count;

	fd = open(path, O_WRONLY);
	if (fd < 0)
		return;

	count = write(fd, text, length);
	if (count != (ssize_t)length) {
		(void)close(fd);
		return;
	}

	(void)close(fd);
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

static void enable_cpu_subtree(const char *group)
{
	char path[PATH_MAX];
	char content[256];

	checked_snprintf(path, sizeof(path), "%s/cgroup.subtree_control",
			 group);
	read_text_file(path, content, sizeof(content));
	if (!has_token(content, "cpu"))
		write_text_file(path, "+cpu\n");
}

static void move_pid_to_cgroup(const char *group, pid_t pid)
{
	char path[PATH_MAX];
	char text[32];

	checked_snprintf(path, sizeof(path), "%s/cgroup.procs", group);
	checked_snprintf(text, sizeof(text), "%d", pid);
	write_text_file(path, text);
}

static void set_weight(const char *group, unsigned int weight)
{
	char path[PATH_MAX];
	char text[32];

	checked_snprintf(path, sizeof(path), "%s/cpu.weight", group);
	checked_snprintf(text, sizeof(text), "%u\n", weight);
	write_text_file(path, text);
}

static void expect_proc_cgroup(pid_t pid, const char *group)
{
	char path[64];
	char content[PATH_MAX];
	char expected[PATH_MAX];
	const char *relative_path;

	if (strncmp(group, CGROUP_ROOT, strlen(CGROUP_ROOT)) != 0)
		fail("cgroup path is outside cgroup root");

	relative_path = group + strlen(CGROUP_ROOT);
	checked_snprintf(path, sizeof(path), "/proc/%d/cgroup", pid);
	read_text_file(path, content, sizeof(content));
	checked_snprintf(expected, sizeof(expected), "0::%s\n", relative_path);

	if (strcmp(content, expected) != 0) {
		fprintf(stderr, "FAIL: %s got \"%s\", expected \"%s\"\n", path,
			content, expected);
		exit(EXIT_FAILURE);
	}
}

static int procs_contains_pid(const char *group, pid_t pid)
{
	char path[PATH_MAX];
	char content[4096];
	char *line;
	char *save_ptr = NULL;

	checked_snprintf(path, sizeof(path), "%s/cgroup.procs", group);
	read_text_file(path, content, sizeof(content));

	line = strtok_r(content, "\n", &save_ptr);
	while (line != NULL) {
		char *end;
		errno = 0;
		long value = strtol(line, &end, 10);

		if (errno == 0 && end != line && *end == '\0' &&
		    value == (long)pid)
			return 1;

		line = strtok_r(NULL, "\n", &save_ptr);
	}

	return 0;
}

static void expect_pid_in_procs(const char *group, pid_t pid)
{
	if (!procs_contains_pid(group, pid)) {
		fprintf(stderr,
			"FAIL: pid %d is missing from %s/cgroup.procs\n", pid,
			group);
		exit(EXIT_FAILURE);
	}
}

static void expect_pid_not_in_procs(const char *group, pid_t pid)
{
	if (procs_contains_pid(group, pid)) {
		fprintf(stderr, "FAIL: pid %d is still in %s/cgroup.procs\n",
			pid, group);
		exit(EXIT_FAILURE);
	}
}

static unsigned long long read_usage_usec(const char *group)
{
	char path[PATH_MAX];
	char content[512];
	char *usage;
	unsigned long long value;

	checked_snprintf(path, sizeof(path), "%s/cpu.stat", group);
	read_text_file(path, content, sizeof(content));

	usage = strstr(content, "usage_usec ");
	if (usage == NULL)
		fail("cpu.stat does not contain usage_usec");

	errno = 0;
	usage += strlen("usage_usec ");
	value = strtoull(usage, NULL, 10);
	if (errno != 0)
		die(path);

	return value;
}

static unsigned long read_populated(const char *group)
{
	char path[PATH_MAX];
	char content[512];
	char *line;
	char *save_ptr = NULL;

	checked_snprintf(path, sizeof(path), "%s/cgroup.events", group);
	read_text_file(path, content, sizeof(content));

	line = strtok_r(content, "\n", &save_ptr);
	while (line != NULL) {
		unsigned long value;
		if (sscanf(line, "populated %lu", &value) == 1)
			return value;
		line = strtok_r(NULL, "\n", &save_ptr);
	}

	fail("cgroup.events does not contain populated");
	return 0;
}

static void expect_populated(const char *group, unsigned long expected)
{
	for (int attempt = 0; attempt < 200; attempt++) {
		if (read_populated(group) == expected)
			return;
		usleep(10000);
	}

	fprintf(stderr,
		"FAIL: %s/cgroup.events populated is %lu, expected %lu\n",
		group, read_populated(group), expected);
	exit(EXIT_FAILURE);
}

static void pin_current_to_cpu0(void)
{
	cpu_set_t mask;

	CPU_ZERO(&mask);
	CPU_SET(0, &mask);
	if (sched_setaffinity(0, sizeof(mask), &mask) < 0)
		die("sched_setaffinity");
}

static void busy_loop_child(void)
{
	volatile unsigned long counter = 0;

	pin_current_to_cpu0();
	if (kill(getpid(), SIGSTOP) < 0)
		_exit(EXIT_FAILURE);

	for (;;) {
		counter++;
		if ((counter & 0xfffffUL) == 0)
			asm volatile("" : : "r"(counter) : "memory");
	}
}

static void sleeping_child_loop(int ready_fd)
{
	char byte = 'x';

	if (write(ready_fd, &byte, 1) != 1)
		_exit(EXIT_FAILURE);
	close(ready_fd);

	for (;;)
		pause();
}

static void wait_for_child_stop(pid_t pid)
{
	int status;

	for (;;) {
		if (waitpid(pid, &status, WUNTRACED) < 0) {
			if (errno == EINTR)
				continue;
			die("waitpid");
		}

		if (!WIFSTOPPED(status))
			fail("child exited before it stopped");
		return;
	}
}

static pid_t spawn_stopped_busy_child(void)
{
	pid_t pid = fork();

	if (pid < 0)
		die("fork");
	if (pid == 0) {
		busy_loop_child();
		_exit(EXIT_FAILURE);
	}

	wait_for_child_stop(pid);
	return pid;
}

static pid_t spawn_sleeping_child(void)
{
	int pipe_fds[2];
	char byte;
	pid_t pid;

	if (pipe(pipe_fds) < 0)
		die("pipe");

	pid = fork();
	if (pid < 0)
		die("fork");
	if (pid == 0) {
		close(pipe_fds[0]);
		sleeping_child_loop(pipe_fds[1]);
		_exit(EXIT_FAILURE);
	}

	close(pipe_fds[1]);
	if (read(pipe_fds[0], &byte, 1) != 1)
		die("read child ready pipe");
	close(pipe_fds[0]);

	return pid;
}

static void continue_child(pid_t pid)
{
	if (kill(pid, SIGCONT) < 0)
		die("SIGCONT");
}

static void kill_and_wait(pid_t *pid)
{
	int status;

	if (*pid <= 0)
		return;

	kill(*pid, SIGKILL);
	for (;;) {
		if (waitpid(*pid, &status, 0) >= 0)
			break;
		if (errno == EINTR)
			continue;
		if (errno == ECHILD)
			break;
		die("waitpid");
	}

	*pid = -1;
}

static void cleanup(void)
{
	char path[PATH_MAX];

	kill_and_wait(&target_pid);
	kill_and_wait(&load_pid);
	kill_and_wait(&sleeping_pid);
	kill_and_wait(&stopped_pid);

	if (child_dir[0] != '\0') {
		checked_snprintf(path, sizeof(path),
				 "%s/cgroup.subtree_control", child_dir);
		try_write_text_file(path, "-cpu\n");
	}
	if (base_dir[0] != '\0') {
		checked_snprintf(path, sizeof(path),
				 "%s/cgroup.subtree_control", base_dir);
		try_write_text_file(path, "-cpu\n");
	}

	if (grandchild_dir[0] != '\0')
		rmdir(grandchild_dir);
	if (peer_dir[0] != '\0')
		rmdir(peer_dir);
	if (load_dir[0] != '\0')
		rmdir(load_dir);
	if (child_dir[0] != '\0')
		rmdir(child_dir);
	if (base_dir[0] != '\0')
		rmdir(base_dir);
}

static void create_cgroup(const char *path)
{
	if (mkdir(path, 0755) < 0)
		die(path);
}

static void create_test_hierarchy(void)
{
	char suffix[64];

	checked_snprintf(suffix, sizeof(suffix), "cglife-%d", getpid());
	checked_snprintf(base_dir, sizeof(base_dir), "%s/%s", CGROUP_ROOT,
			 suffix);
	checked_snprintf(child_dir, sizeof(child_dir), "%s/child", base_dir);
	checked_snprintf(grandchild_dir, sizeof(grandchild_dir),
			 "%s/grandchild", child_dir);
	checked_snprintf(load_dir, sizeof(load_dir), "%s/load", base_dir);
	checked_snprintf(peer_dir, sizeof(peer_dir), "%s/peer", base_dir);

	enable_cpu_subtree(CGROUP_ROOT);
	create_cgroup(base_dir);
	create_cgroup(child_dir);
	create_cgroup(load_dir);
	create_cgroup(peer_dir);
	enable_cpu_subtree(base_dir);
}

static void test_stopped_migration(void)
{
	stopped_pid = spawn_stopped_busy_child();

	move_pid_to_cgroup(child_dir, stopped_pid);
	expect_proc_cgroup(stopped_pid, child_dir);
	expect_pid_in_procs(child_dir, stopped_pid);
	expect_populated(child_dir, 1);

	kill_and_wait(&stopped_pid);
	expect_populated(child_dir, 0);
}

static void test_sleeping_grandchild_migration(void)
{
	enable_cpu_subtree(child_dir);
	create_cgroup(grandchild_dir);

	sleeping_pid = spawn_sleeping_child();
	move_pid_to_cgroup(grandchild_dir, sleeping_pid);
	expect_proc_cgroup(sleeping_pid, grandchild_dir);
	expect_pid_in_procs(grandchild_dir, sleeping_pid);
	expect_populated(child_dir, 1);
	expect_populated(grandchild_dir, 1);

	kill_and_wait(&sleeping_pid);
	expect_populated(grandchild_dir, 0);
}

static void test_runnable_migration_under_load(void)
{
	load_pid = spawn_stopped_busy_child();
	move_pid_to_cgroup(load_dir, load_pid);
	continue_child(load_pid);

	target_pid = spawn_stopped_busy_child();
	move_pid_to_cgroup(grandchild_dir, target_pid);
	continue_child(target_pid);

	sleep(1);

	move_pid_to_cgroup(peer_dir, target_pid);
	expect_proc_cgroup(target_pid, peer_dir);
	expect_pid_in_procs(peer_dir, target_pid);
	expect_pid_not_in_procs(grandchild_dir, target_pid);
	expect_populated(peer_dir, 1);
	expect_populated(grandchild_dir, 0);

	kill_and_wait(&target_pid);
	kill_and_wait(&load_pid);
	expect_populated(peer_dir, 0);
	expect_populated(load_dir, 0);
	expect_populated(child_dir, 0);
}

static void test_runnable_migration_uses_new_weight(void)
{
	unsigned long long load_before;
	unsigned long long load_delta;
	unsigned long long peer_before;
	unsigned long long peer_delta;
	unsigned long long total_delta;
	unsigned int peer_share_bp;

	set_weight(load_dir, 100);
	set_weight(peer_dir, 10000);

	load_pid = spawn_stopped_busy_child();
	move_pid_to_cgroup(load_dir, load_pid);
	continue_child(load_pid);

	target_pid = spawn_stopped_busy_child();
	move_pid_to_cgroup(load_dir, target_pid);
	continue_child(target_pid);

	sleep(1);

	load_before = read_usage_usec(load_dir);
	peer_before = read_usage_usec(peer_dir);

	move_pid_to_cgroup(peer_dir, target_pid);
	expect_proc_cgroup(target_pid, peer_dir);
	expect_pid_in_procs(peer_dir, target_pid);
	expect_pid_not_in_procs(load_dir, target_pid);

	sleep(MIGRATION_WINDOW_SECONDS);

	load_delta = read_usage_usec(load_dir) - load_before;
	peer_delta = read_usage_usec(peer_dir) - peer_before;
	total_delta = load_delta + peer_delta;
	if (total_delta < MIN_MIGRATION_TOTAL_USEC)
		fail("migration weight test collected too little CPU time");

	peer_share_bp = (unsigned int)(peer_delta * 10000 / total_delta);
	if (peer_share_bp < 7000) {
		fprintf(stderr,
			"FAIL: migrated task share=%u.%02u%%, expected at least 70%%\n",
			peer_share_bp / 100, peer_share_bp % 100);
		exit(EXIT_FAILURE);
	}

	fprintf(stderr, "migrated task share=%u.%02u%%\n", peer_share_bp / 100,
		peer_share_bp % 100);

	kill_and_wait(&target_pid);
	kill_and_wait(&load_pid);
	expect_populated(peer_dir, 0);
	expect_populated(load_dir, 0);
}

int main(void)
{
	atexit(cleanup);
	pin_current_to_cpu0();
	create_test_hierarchy();

	test_stopped_migration();
	test_sleeping_grandchild_migration();
	test_runnable_migration_under_load();
	test_runnable_migration_uses_new_weight();

	fprintf(stderr, "cgroup lifecycle stress regression completed\n");
	return 0;
}
