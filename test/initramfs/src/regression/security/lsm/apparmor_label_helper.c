// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include "apparmor_label_common.h"
#include <pthread.h>
#include <sys/syscall.h>
#include <sys/wait.h>

extern char **environ;

static void check_self(void)
{
	char label[288], path[128];
	read_label("/proc/self/attr/current", label, sizeof(label));
	assert(strcmp(label, getenv("AA_EXPECT")) == 0);
	read_label("/proc/thread-self/attr/current", label, sizeof(label));
	assert(strcmp(label, getenv("AA_EXPECT")) == 0);
	snprintf(path, sizeof(path), "/proc/%d/task/%ld/attr/current", getpid(),
		 syscall(SYS_gettid));
	read_label(path, label, sizeof(label));
	assert(strcmp(label, getenv("AA_EXPECT")) == 0);
}

static void *thread_check(void *unused)
{
	(void)unused;
	check_self();
	return NULL;
}

static void *thread_exec(void *unused)
{
	(void)unused;
	assert(setenv("AA_ACTION", "hold", 1) == 0);
	execl(AA_TEST_DIR "/a", "misleading-argv-zero", NULL);
	_exit(90);
}

int main(void)
{
	const char *action = getenv("AA_ACTION");
	assert(action && getenv("AA_EXPECT"));
	if (strcmp(action, "thread-exec") == 0) {
		pthread_t thread;
		assert(pthread_create(&thread, NULL, thread_exec, NULL) == 0);
		for (;;)
			pause();
	}
	check_self();
	if (strcmp(action, "leaf") == 0)
		return 0;
	if (strcmp(action, "reexec") == 0) {
		assert(setenv("AA_ACTION", "hold", 1) == 0);
		execl(AA_TEST_DIR "/b", "untrusted-argv-zero", NULL);
		return 91;
	}
	if (strcmp(action, "failed-exec") == 0 ||
	    strcmp(action, "late-fail") == 0) {
		execl(AA_TEST_DIR "/broken", "broken", NULL);
		assert(errno ==
		       (!strcmp(action, "late-fail") ? ENOENT : ENOEXEC));
		check_self();
	}
	if (strcmp(action, "inherit") == 0) {
		pthread_t thread;
		assert(pthread_create(&thread, NULL, thread_check, NULL) == 0);
		assert(pthread_join(thread, NULL) == 0);
		pid_t child = fork();
		assert(child >= 0);
		if (!child) {
			check_self();
			_exit(0);
		}
		int status;
		assert(waitpid(child, &status, 0) == child);
		assert(WIFEXITED(status) && WEXITSTATUS(status) == 0);
		assert(setenv("AA_ACTION", "leaf", 1) == 0);
		char *args[] = { "vfork-reexec", NULL };
		child = vfork();
		if (!child) {
			execve(AA_TEST_DIR "/b", args, environ);
			_exit(92);
		}
		assert(child > 0);
		assert(waitpid(child, &status, 0) == child);
		assert(WIFEXITED(status) && WEXITSTATUS(status) == 0);
		check_self();
	}

	struct label_report report = { .pid = getpid() };
	read_label("/proc/self/attr/current", report.label,
		   sizeof(report.label));
	int ready = atoi(getenv("AA_READY"));
	int release = atoi(getenv("AA_RELEASE"));
	transfer(ready, &report, sizeof(report), 1);
	assert(close(ready) == 0);
	char go;
	transfer(release, &go, 1, 0);
	assert(close(release) == 0);
	return 0;
}
