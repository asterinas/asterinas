// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include "../../common/test.h"
#include "apparmor_label_common.h"
#include <elf.h>
#include <pthread.h>
#include <sys/mman.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/wait.h>

extern char **environ;

static void copy_to_fd(const char *source, int output)
{
	int input = CHECK(open(source, O_RDONLY));
	char buf[8192];
	ssize_t size;
	while ((size = CHECK(read(input, buf, sizeof(buf)))))
		transfer(output, buf, size, 1);
	CHECK(close(input));
}

static void copy_file(const char *source, const char *target)
{
	int output = CHECK(open(target, O_WRONLY | O_CREAT | O_TRUNC, 0755));
	copy_to_fd(source, output);
	CHECK(close(output));
}

static void load_fixture(const char *name)
{
	char path[256];
	snprintf(path, sizeof(path), "/test/security/lsm/apparmor-fixtures/%s",
		 name);
	int input = CHECK(open(path, O_RDONLY));
	struct stat st;
	CHECK(fstat(input, &st));
	void *blob = CHECK_WITH(malloc(st.st_size), _ret != NULL);
	transfer(input, blob, st.st_size, 0);
	CHECK(close(input));
	int output =
		CHECK(open("/sys/kernel/security/apparmor/.load", O_WRONLY));
	CHECK_WITH(write(output, blob, st.st_size), _ret == st.st_size);
	CHECK(close(output));
	free(blob);
}

/* A valid ELF with a missing interpreter fails after label preparation. */
static void make_late_failure(void)
{
	copy_file("/proc/self/exe", AA_TEST_DIR "/broken");
	int fd = CHECK(open(AA_TEST_DIR "/broken", O_RDWR));
	Elf64_Ehdr header;
	CHECK_WITH(pread(fd, &header, sizeof(header), 0),
		   _ret == sizeof(header));
	assert(header.e_phentsize == sizeof(Elf64_Phdr));
	for (size_t i = 0; i < header.e_phnum; i++) {
		Elf64_Phdr phdr;
		CHECK_WITH(pread(fd, &phdr, sizeof(phdr),
				 header.e_phoff + i * sizeof(phdr)),
			   _ret == sizeof(phdr));
		if (phdr.p_type != PT_INTERP)
			continue;
		assert(phdr.p_filesz > 2 && phdr.p_filesz < 4096);
		char path[4096];
		memset(path, 'x', phdr.p_filesz);
		path[0] = '/';
		path[phdr.p_filesz - 1] = '\0';
		CHECK_WITH(pwrite(fd, path, phdr.p_filesz, phdr.p_offset),
			   _ret == phdr.p_filesz);
		CHECK(close(fd));
		return;
	}
	assert(!"test executable must have PT_INTERP");
}

struct child {
	pid_t pid;
	int ready;
	int release;
	int before_exec;
};

struct query_race {
	pthread_barrier_t barrier;
	pid_t pid;
	const char *expected;
};

static void *query_during_exec(void *argument)
{
	struct query_race *race = argument;
	char path[128], label[288];
	snprintf(path, sizeof(path), "/proc/%d/attr/current", race->pid);
	read_label(path, label, sizeof(label));
	assert(!strcmp(label, "unconfined\n"));
	pthread_barrier_wait(&race->barrier);
	for (size_t i = 0; i < 256; i++) {
		read_label(path, label, sizeof(label));
		assert(!strcmp(label, "unconfined\n") ||
		       !strcmp(label, race->expected));
		sched_yield();
	}
	return NULL;
}

static struct child start_child(const char *path, const char *expected,
				const char *action, const char *style)
{
	int ready[2], release[2], gate[2];
	CHECK(pipe(ready));
	CHECK(pipe(release));
	CHECK(pipe(gate));
	char number[32];
	snprintf(number, sizeof(number), "%d", ready[1]);
	CHECK(setenv("AA_READY", number, 1));
	snprintf(number, sizeof(number), "%d", release[0]);
	CHECK(setenv("AA_RELEASE", number, 1));
	CHECK(setenv("AA_EXPECT", expected, 1));
	CHECK(setenv("AA_ACTION", action, 1));
	pid_t pid = CHECK(fork());
	if (!pid) {
		CHECK(close(ready[0]));
		CHECK(close(release[1]));
		CHECK(close(gate[1]));
		char go;
		transfer(gate[0], &go, 1, 0);
		CHECK(close(gate[0]));
		char *args[] = { "argv-zero-is-not-a-path", NULL };
		if (!strcmp(style, "relative")) {
			CHECK(chdir(AA_TEST_DIR));
			path = "./a";
		} else if (!strcmp(style, "chroot")) {
			CHECK(chroot(AA_TEST_DIR "/root"));
			CHECK(chdir("/"));
			path = "/bin/a";
		} else if (!strcmp(style, "uid")) {
			CHECK(setgid(65534));
			CHECK(setuid(65534));
		}
		if (!strcmp(style, "fd") || !strcmp(style, "deleted") ||
		    !strcmp(style, "memfd") || !strcmp(style, "detached")) {
			int fd;
			if (!strcmp(style, "memfd")) {
				fd = CHECK(memfd_create("aa-anonymous", 0));
				copy_to_fd(AA_HELPER, fd);
				CHECK(fchmod(fd, 0755));
			} else {
				fd = CHECK(open(path, O_RDONLY));
			}
			if (!strcmp(style, "deleted"))
				CHECK(unlink(path));
			if (!strcmp(style, "detached"))
				CHECK(umount2(AA_TEST_DIR "/u", MNT_DETACH));
			syscall(SYS_execveat, fd, "", args, environ,
				AT_EMPTY_PATH);
		} else {
			execve(path, args, environ);
		}
		perror("exec label helper");
		_exit(93);
	}
	CHECK(close(ready[1]));
	CHECK(close(release[0]));
	CHECK(close(gate[0]));
	char proc[128];
	snprintf(proc, sizeof(proc), "/proc/%d/attr/current", pid);
	int before = CHECK(open(proc, O_RDONLY));
	struct query_race race = { .pid = pid, .expected = expected };
	pthread_t query_thread;
	if (!strcmp(style, "race")) {
		assert(pthread_barrier_init(&race.barrier, NULL, 2) == 0);
		assert(pthread_create(&query_thread, NULL, query_during_exec,
				      &race) == 0);
		pthread_barrier_wait(&race.barrier);
	}
	char go = 'g';
	transfer(gate[1], &go, 1, 1);
	CHECK(close(gate[1]));
	if (!strcmp(style, "race")) {
		assert(pthread_join(query_thread, NULL) == 0);
		assert(pthread_barrier_destroy(&race.barrier) == 0);
	}
	return (struct child){ pid, ready[0], release[1], before };
}

static void observe_child(struct child child, const char *expected)
{
	struct label_report report;
	transfer(child.ready, &report, sizeof(report), 0);
	CHECK(close(child.ready));
	assert(report.pid == child.pid);
	assert(!strcmp(report.label, expected));
	char proc[128], label[288];
	snprintf(proc, sizeof(proc), "/proc/%d/attr/current", child.pid);
	int current = CHECK(open(proc, O_RDONLY));
	read_label_fd(current, label, sizeof(label));
	assert(!strcmp(label, expected));
	read_label_fd(child.before_exec, label, sizeof(label));
	assert(!strcmp(label, "unconfined\n"));
	CHECK(close(child.before_exec));

	/* A caller with a different UID and no ptrace capability is denied. */
	pid_t denied = CHECK(fork());
	if (!denied) {
		CHECK(setgid(65533));
		CHECK(setuid(65533));
		int fd = open(proc, O_RDONLY);
		_exit(fd == -1 && errno == EACCES ? 0 : 94);
	}
	int status;
	CHECK(waitpid(denied, &status, 0));
	assert(WIFEXITED(status) && WEXITSTATUS(status) == 0);

	char go = 'x';
	transfer(child.release, &go, 1, 1);
	CHECK(close(child.release));
	CHECK(waitpid(child.pid, &status, 0));
	assert(WIFEXITED(status) && WEXITSTATUS(status) == 0);
	/* The authorized open snapshot survives target exit and PID disappearance. */
	read_label_fd(current, label, sizeof(label));
	assert(!strcmp(label, expected));
	CHECK(close(current));
	assert(open(proc, O_RDONLY) == -1);
	assert(errno == ENOENT || errno == ESRCH);
	read_label("/proc/self/attr/current", label, sizeof(label));
	assert(!strcmp(label, "unconfined\n"));
}

FN_TEST(exec_binding_and_pid_queries)
{
	if (access("/sys/kernel/security/apparmor/.load", F_OK)) {
		TEST_ERRNO(open("/proc/self/attr/current", O_RDONLY), EINVAL);
		return;
	}
	int writable = open("/proc/self/attr/current", O_WRONLY);
	if (writable >= 0) {
		TEST_ERRNO(write(writable, "unconfined", 10), EPERM);
		CHECK(close(writable));
	} else {
		TEST_RES(errno, _ret == EACCES || _ret == EPERM);
	}
	CHECK(mkdir(AA_TEST_DIR, 0755));
	copy_file(AA_HELPER, AA_TEST_DIR "/a");
	copy_file(AA_HELPER, AA_TEST_DIR "/b");
	copy_file(AA_HELPER, AA_TEST_DIR "/u");
	CHECK(link(AA_TEST_DIR "/a", AA_TEST_DIR "/hardlink"));
	CHECK(link(AA_TEST_DIR "/a", AA_TEST_DIR "/deleted"));
	CHECK(symlink(AA_TEST_DIR "/a", AA_TEST_DIR "/link"));
	int fd = CHECK(open(AA_TEST_DIR "/broken", O_CREAT | O_WRONLY, 0755));
	CHECK(write(fd, "not an ELF", 10));
	CHECK(close(fd));
	fd = CHECK(open(AA_TEST_DIR "/script", O_CREAT | O_WRONLY, 0755));
	const char script[] = "#!" AA_TEST_DIR "/b\n";
	CHECK(write(fd, script, sizeof(script) - 1));
	CHECK(close(fd));

	/* Loading policy must not relabel an already executing task. */
	struct child old =
		start_child(AA_TEST_DIR "/a", "unconfined\n", "hold", "normal");
	struct label_report ready;
	transfer(old.ready, &ready, sizeof(ready), 0);
	assert(!strcmp(ready.label, "unconfined\n"));
	const char *fixtures[] = { "label-a.bin", "label-b.bin", "label-script.bin",
				   "label-broken.bin", "label-deleted.bin", "label-chroot.bin" };
	for (size_t i = 0; i < sizeof(fixtures) / sizeof(fixtures[0]); i++)
		load_fixture(fixtures[i]);
	char proc[128], label[288];
	snprintf(proc, sizeof(proc), "/proc/%d/attr/current", old.pid);
	read_label(proc, label, sizeof(label));
	TEST_RES(strcmp(label, "unconfined\n"), _ret == 0);
	char go = 'x';
	transfer(old.release, &go, 1, 1);
	CHECK(close(old.ready));
	CHECK(close(old.release));
	CHECK(close(old.before_exec));
	int status;
	CHECK(waitpid(old.pid, &status, 0));
	TEST_RES(status, _ret == 0);

	/* Keep all three targets alive at once; query by each actual PID. */
	struct child a = start_child(AA_TEST_DIR "/a", "m2-a (enforce)\n",
				     "hold", "normal");
	struct child b = start_child(AA_TEST_DIR "/b", "m2-b (complain)\n",
				     "hold", "normal");
	struct child u =
		start_child(AA_TEST_DIR "/u", "unconfined\n", "hold", "normal");
	observe_child(a, "m2-a (enforce)\n");
	observe_child(b, "m2-b (complain)\n");
	observe_child(u, "unconfined\n");
	TEST_SUCC(0);

	const struct {
		const char *path, *label, *action, *style;
	} cases[] = {
		{ AA_TEST_DIR "/a", "m2-a (enforce)\n", "inherit", "normal" },
		{ AA_TEST_DIR "/a", "m2-a (enforce)\n", "reexec", "normal" },
		{ AA_TEST_DIR "/a", "m2-a (enforce)\n", "failed-exec",
		  "normal" },
		{ AA_TEST_DIR "/u", "unconfined\n", "failed-exec", "normal" },
		{ AA_TEST_DIR "/a", "m2-a (enforce)\n", "hold", "relative" },
		{ AA_TEST_DIR "/a", "m2-a (enforce)\n", "hold", "race" },
		{ AA_TEST_DIR "/link", "m2-a (enforce)\n", "hold", "normal" },
		{ AA_TEST_DIR "/hardlink", "unconfined\n", "hold", "normal" },
		{ AA_TEST_DIR "/a", "m2-a (enforce)\n", "hold", "fd" },
		{ AA_TEST_DIR "/deleted", "unconfined\n", "hold", "deleted" },
		{ AA_TEST_DIR "/u", "unconfined\n", "hold", "memfd" },
		{ AA_TEST_DIR "/script", "m2-script (enforce)\n", "hold",
		  "normal" },
		{ AA_TEST_DIR "/a", "m2-a (enforce)\n", "hold", "uid" },
		{ AA_TEST_DIR "/u", "m2-a (enforce)\n", "thread-exec",
		  "normal" },
	};
	for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
		printf("label case %zu: %s / %s\n", i, cases[i].action,
		       cases[i].style);
		observe_child(start_child(cases[i].path, cases[i].label,
					  cases[i].action, cases[i].style),
			      cases[i].label);
		TEST_SUCC(0);
	}
	/* The opened file belongs to a mount detached from this namespace. */
	CHECK(mount(AA_TEST_DIR "/a", AA_TEST_DIR "/u", NULL, MS_BIND, NULL));
	observe_child(start_child(AA_TEST_DIR "/u", "unconfined\n", "hold",
				  "detached"),
		      "unconfined\n");
	TEST_SUCC(0);
	make_late_failure();
	observe_child(start_child(AA_TEST_DIR "/u", "unconfined\n", "late-fail",
				  "normal"),
		      "unconfined\n");
	observe_child(start_child(AA_TEST_DIR "/a", "m2-a (enforce)\n",
				  "late-fail", "normal"),
		      "m2-a (enforce)\n");
	TEST_SUCC(0);

	CHECK(mkdir(AA_TEST_DIR "/root", 0755));
	CHECK(mkdir(AA_TEST_DIR "/root/bin", 0755));
	CHECK(mkdir(AA_TEST_DIR "/root/proc", 0755));
	copy_file(AA_HELPER, AA_TEST_DIR "/root/bin/a");
	CHECK(mount("/proc", AA_TEST_DIR "/root/proc", NULL, MS_BIND, NULL));
	observe_child(start_child("/bin/a", "m2-chroot (enforce)\n", "hold",
				  "chroot"),
		      "m2-chroot (enforce)\n");
	TEST_SUCC(0);
	CHECK(umount(AA_TEST_DIR "/root/proc"));
	CHECK(unlink(AA_TEST_DIR "/root/bin/a"));
	CHECK(rmdir(AA_TEST_DIR "/root/proc"));
	CHECK(rmdir(AA_TEST_DIR "/root/bin"));
	CHECK(rmdir(AA_TEST_DIR "/root"));
	const char *files[] = { "a",	    "b",      "u",     "link",
				"hardlink", "broken", "script" };
	for (size_t i = 0; i < sizeof(files) / sizeof(files[0]); i++) {
		snprintf(proc, sizeof(proc), AA_TEST_DIR "/%s", files[i]);
		CHECK(unlink(proc));
	}
	CHECK(rmdir(AA_TEST_DIR));
}
END_TEST()
