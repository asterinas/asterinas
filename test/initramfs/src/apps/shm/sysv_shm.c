// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <setjmp.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ipc.h>
#include <sys/mman.h>
#include <sys/shm.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

static size_t page_size;
static size_t segment_size;
static sigjmp_buf fault_env;
static volatile sig_atomic_t fault_armed;

enum fault_op {
	FAULT_READ,
	FAULT_WRITE,
};

static const char *fault_op_name(enum fault_op op)
{
	return op == FAULT_WRITE ? "writing" : "reading";
}

static void print_case_header(const char *title)
{
	printf("\n==== %s ====\n", title);
}

static void fault_handler(int signo)
{
	if (fault_armed) {
		siglongjmp(fault_env, signo);
	}
	const char msg[] = "unexpected fault outside probe; aborting\n";
	(void)!write(STDERR_FILENO, msg, sizeof(msg) - 1);
	_exit(128 + signo);
}

static void install_fault_handlers(void)
{
	struct sigaction sa = {
		.sa_handler = fault_handler,
		.sa_flags = SA_NODEFER,
	};
	sigemptyset(&sa.sa_mask);
	if (sigaction(SIGSEGV, &sa, NULL) < 0 ||
	    sigaction(SIGBUS, &sa, NULL) < 0) {
		perror("sigaction");
		exit(EXIT_FAILURE);
	}
}

static key_t next_key(void)
{
	static unsigned int salt = 0;
	return (key_t)(0x24680000 ^ (unsigned)getpid() ^ salt++);
}

static void dirty_all_pages(unsigned char *addr)
{
	for (size_t offset = 0; offset < segment_size; offset += page_size) {
		addr[offset] = (unsigned char)(offset / page_size);
	}
}

static void report_shm_nattch(const char *label, int shmid)
{
	struct shmid_ds ds;
	errno = 0;
	int ret = shmctl(shmid, IPC_STAT, &ds);
	if (ret < 0) {
		printf("  %s -> shmctl(IPC_STAT) ret=%d errno=%d (%s)\n", label,
		       ret, errno, strerror(errno));
	} else {
		printf("  %s -> shmctl(IPC_STAT) nattch=%lu\n", label,
		       (unsigned long)ds.shm_nattch);
	}
}

static unsigned long get_shm_nattch_or_die(const char *label, int shmid)
{
	struct shmid_ds ds;
	errno = 0;
	if (shmctl(shmid, IPC_STAT, &ds) < 0) {
		fprintf(stderr,
			"%s: shmctl(IPC_STAT, shmid=%d) failed errno=%d (%s)\n",
			label, shmid, errno, strerror(errno));
		exit(EXIT_FAILURE);
	}
	return (unsigned long)ds.shm_nattch;
}

static void expect_shm_nattch_eq(const char *label, int shmid,
				 unsigned long expected)
{
	unsigned long got = get_shm_nattch_or_die(label, shmid);
	if (got != expected) {
		fprintf(stderr,
			"%s: unexpected shm_nattch for shmid=%d: got=%lu expected=%lu\n",
			label, shmid, got, expected);
		exit(EXIT_FAILURE);
	}
}

static void write_full_or_die(int fd, const void *buf, size_t len)
{
	const unsigned char *p = (const unsigned char *)buf;
	size_t written = 0;
	while (written < len) {
		ssize_t n = write(fd, p + written, len - written);
		if (n < 0) {
			if (errno == EINTR) {
				continue;
			}
			perror("write");
			exit(EXIT_FAILURE);
		}
		if (n == 0) {
			fprintf(stderr, "write: unexpected EOF\n");
			exit(EXIT_FAILURE);
		}
		written += (size_t)n;
	}
}

static void read_full_or_die(int fd, void *buf, size_t len)
{
	unsigned char *p = (unsigned char *)buf;
	size_t read_bytes = 0;
	while (read_bytes < len) {
		ssize_t n = read(fd, p + read_bytes, len - read_bytes);
		if (n < 0) {
			if (errno == EINTR) {
				continue;
			}
			perror("read");
			exit(EXIT_FAILURE);
		}
		if (n == 0) {
			fprintf(stderr, "read: unexpected EOF\n");
			exit(EXIT_FAILURE);
		}
		read_bytes += (size_t)n;
	}
}

static int probe_access(volatile char *addr, enum fault_op op)
{
	fault_armed = 1;
	int signo = sigsetjmp(fault_env, 1);
	if (signo == 0) {
		if (op == FAULT_WRITE) {
			*addr = (char)0xff;
		} else {
			(void)*addr;
		}
		fault_armed = 0;
		return 0;
	}
	fault_armed = 0;
	return signo;
}

static void trigger_fault_probe(const char *label, volatile char *addr,
				enum fault_op op)
{
	fault_armed = 1;
	int signo = sigsetjmp(fault_env, 1);
	if (signo == 0) {
		if (op == FAULT_WRITE) {
			*addr = (char)0xff;
		} else {
			(void)*addr;
		}
		fault_armed = 0;
		printf("  %s -> no signal when %s at %p\n", label,
		       fault_op_name(op), (const void *)addr);
		return;
	}
	fault_armed = 0;
	printf("  %s -> received signal %d (%s) while %s at %p\n", label, signo,
	       strsignal(signo), fault_op_name(op), (const void *)addr);
}

static void probe_read_value(const char *label, volatile unsigned char *addr)
{
	fault_armed = 1;
	int signo = sigsetjmp(fault_env, 1);
	if (signo == 0) {
		unsigned char value = *addr;
		fault_armed = 0;
		printf("  %s -> read 0x%02x at %p\n", label, value,
		       (const void *)addr);
		return;
	}
	fault_armed = 0;
	printf("  %s -> received signal %d (%s) while reading at %p\n", label,
	       signo, strsignal(signo), (const void *)addr);
}

static void scenario_ipc_rmid_existing_mapping(void)
{
	print_case_header(
		"IPC_RMID: existing mapping stays usable, new attach fails");

	key_t key = next_key();
	errno = 0;
	int shmid = shmget(key, segment_size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget(key=0x%lx) -> shmid=%d errno=%d (%s)\n",
	       (unsigned long)key, shmid, errno, strerror(errno));
	if (shmid < 0) {
		return;
	}

	errno = 0;
	unsigned char *addr = shmat(shmid, NULL, 0);
	printf("  shmat(NULL) -> addr=%p errno=%d (%s)\n", addr, errno,
	       strerror(errno));
	if (addr == (void *)-1) {
		errno = 0;
		int rc = shmctl(shmid, IPC_RMID, NULL);
		printf("  shmctl(IPC_RMID) -> ret=%d errno=%d (%s)\n", rc,
		       errno, strerror(errno));
		return;
	}

	addr[0] = 0x5a;
	addr[1] = 0;
	printf("  initial bytes -> [%p]={0x%02x,0x%02x}\n", addr, addr[0],
	       addr[1]);

	errno = 0;
	int rc = shmctl(shmid, IPC_RMID, NULL);
	printf("  shmctl(IPC_RMID) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));

	addr[1] = 0xa5;
	printf("  write after IPC_RMID -> byte1=0x%02x\n", addr[1]);

	errno = 0;
	void *second = shmat(shmid, NULL, 0);
	printf("  shmat() after IPC_RMID -> addr=%p errno=%d (%s)\n", second,
	       errno, strerror(errno));
	if (second != (void *)-1) {
		shmdt(second);
	}

	errno = 0;
	rc = shmdt(addr);
	printf("  shmdt(original) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
}

static void scenario_fork_shared_writes(void)
{
	print_case_header("fork shares writable pages (no COW)");

	errno = 0;
	int shmid =
		shmget(IPC_PRIVATE, segment_size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget(IPC_PRIVATE) -> shmid=%d errno=%d (%s)\n", shmid,
	       errno, strerror(errno));
	if (shmid < 0) {
		return;
	}

	errno = 0;
	int *addr = shmat(shmid, NULL, 0);
	printf("  shmat(NULL) -> addr=%p errno=%d (%s)\n", addr, errno,
	       strerror(errno));
	if (addr == (void *)-1) {
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}
	addr[0] = 0;

	pid_t pid = fork();
	if (pid < 0) {
		perror("fork");
		shmdt(addr);
		shmctl(shmid, IPC_RMID, NULL);
		return;
	} else if (pid == 0) {
		addr[0] = 0x12345678;
		printf("  [child] wrote value 0x%x\n", addr[0]);
		// Pirnt current shared memory nattch
		report_shm_nattch("  [child] before exit", shmid);
		_exit(0);
	}

	int status = 0;
	waitpid(pid, &status, 0);
	printf("  [parent] waitpid status=0x%x value observed=0x%x\n", status,
	       addr[0]);

	shmdt(addr);
	errno = 0;
	int rc = shmctl(shmid, IPC_RMID, NULL);
	printf("  shmctl(IPC_RMID) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
}

static void scenario_fork_refcnt_multi_attach(void)
{
	print_case_header("fork increments nattch for multiple attachments");

	const size_t size = page_size;
	errno = 0;
	int shmid = shmget(IPC_PRIVATE, size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget(IPC_PRIVATE) -> shmid=%d errno=%d (%s)\n", shmid,
	       errno, strerror(errno));
	if (shmid < 0) {
		exit(EXIT_FAILURE);
	}

	void *addrs[3] = { NULL, NULL, NULL };
	for (size_t i = 0; i < 3; i++) {
		errno = 0;
		addrs[i] = shmat(shmid, NULL, 0);
		printf("  shmat #%zu -> addr=%p errno=%d (%s)\n", i, addrs[i],
		       errno, strerror(errno));
		if (addrs[i] == (void *)-1) {
			exit(EXIT_FAILURE);
		}
	}
	if (addrs[0] == addrs[1] || addrs[0] == addrs[2] ||
	    addrs[1] == addrs[2]) {
		fprintf(stderr, "shmat returned duplicated addresses\n");
		exit(EXIT_FAILURE);
	}

	expect_shm_nattch_eq("  before fork", shmid, 3);

	int c2p[2];
	int p2c[2];
	if (pipe(c2p) < 0 || pipe(p2c) < 0) {
		perror("pipe");
		exit(EXIT_FAILURE);
	}

	pid_t pid = fork();
	if (pid < 0) {
		perror("fork");
		exit(EXIT_FAILURE);
	}

	if (pid == 0) {
		close(c2p[0]);
		close(p2c[1]);

		unsigned long after_fork =
			get_shm_nattch_or_die("  [child] after fork", shmid);
		write_full_or_die(c2p[1], &after_fork, sizeof(after_fork));

		char cmd = 0;
		read_full_or_die(p2c[0], &cmd, 1);

		for (size_t i = 0; i < 3; i++) {
			errno = 0;
			int rc = shmdt(addrs[i]);
			printf("  [child] shmdt #%zu -> ret=%d errno=%d (%s)\n",
			       i, rc, errno, strerror(errno));
			if (rc < 0) {
				_exit(1);
			}
		}

		unsigned long after_detach =
			get_shm_nattch_or_die("  [child] after detach", shmid);
		write_full_or_die(c2p[1], &after_detach, sizeof(after_detach));
		_exit(0);
	}

	close(c2p[1]);
	close(p2c[0]);

	unsigned long child_after_fork = 0;
	read_full_or_die(c2p[0], &child_after_fork, sizeof(child_after_fork));
	printf("  [parent] child reported nattch=%lu after fork\n",
	       child_after_fork);

	unsigned long parent_after_fork =
		get_shm_nattch_or_die("  [parent] after fork", shmid);
	printf("  [parent] observed nattch=%lu after fork\n",
	       parent_after_fork);

	if (child_after_fork != 6 || parent_after_fork != 6) {
		fprintf(stderr,
			"unexpected nattch after fork: child=%lu parent=%lu expected=6\n",
			child_after_fork, parent_after_fork);
		exit(EXIT_FAILURE);
	}

	write_full_or_die(p2c[1], "D", 1);

	unsigned long child_after_detach = 0;
	read_full_or_die(c2p[0], &child_after_detach,
			 sizeof(child_after_detach));
	printf("  [parent] child reported nattch=%lu after detach\n",
	       child_after_detach);

	int status = 0;
	waitpid(pid, &status, 0);
	printf("  [parent] waitpid status=0x%x\n", status);
	if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
		fprintf(stderr, "child exited abnormally\n");
		exit(EXIT_FAILURE);
	}

	expect_shm_nattch_eq("  after child detach+exit", shmid, 3);

	for (size_t i = 0; i < 3; i++) {
		errno = 0;
		int rc = shmdt(addrs[i]);
		printf("  [parent] shmdt #%zu -> ret=%d errno=%d (%s)\n", i, rc,
		       errno, strerror(errno));
		if (rc < 0) {
			exit(EXIT_FAILURE);
		}
	}

	expect_shm_nattch_eq("  after parent detaches all", shmid, 0);

	errno = 0;
	int rc = shmctl(shmid, IPC_RMID, NULL);
	printf("  shmctl(IPC_RMID) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
}

static void scenario_readonly_attach_no_upgrade(void)
{
	print_case_header("SHM_RDONLY attach cannot regain write via mprotect");

	errno = 0;
	int shmid =
		shmget(IPC_PRIVATE, segment_size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget -> shmid=%d errno=%d (%s)\n", shmid, errno,
	       strerror(errno));
	if (shmid < 0) {
		return;
	}

	errno = 0;
	char *addr = shmat(shmid, NULL, SHM_RDONLY);
	printf("  shmat(SHM_RDONLY) -> addr=%p errno=%d (%s)\n", addr, errno,
	       strerror(errno));
	if (addr == (void *)-1) {
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	errno = 0;
	int rc = mprotect(addr, segment_size, PROT_READ | PROT_WRITE);
	printf("  mprotect(PROT_READ|PROT_WRITE) -> ret=%d errno=%d (%s)\n", rc,
	       errno, strerror(errno));

	trigger_fault_probe("  write attempt after read-only attach", addr,
			    FAULT_WRITE);

	shmdt(addr);
	errno = 0;
	rc = shmctl(shmid, IPC_RMID, NULL);
	printf("  shmctl(IPC_RMID) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
}

static void scenario_mprotect_downgrade_faults(void)
{
	print_case_header("mprotect downgrade to READ/NONE enforces faults");

	errno = 0;
	int shmid =
		shmget(IPC_PRIVATE, segment_size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget -> shmid=%d errno=%d (%s)\n", shmid, errno,
	       strerror(errno));
	if (shmid < 0) {
		return;
	}

	errno = 0;
	char *addr = shmat(shmid, NULL, 0);
	printf("  shmat -> addr=%p errno=%d (%s)\n", addr, errno,
	       strerror(errno));
	if (addr == (void *)-1) {
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}
	memset(addr, 0xab, segment_size);

	errno = 0;
	int rc = mprotect(addr, segment_size, PROT_READ);
	printf("  mprotect(PROT_READ) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
	trigger_fault_probe("  write after PROT_READ", addr, FAULT_WRITE);

	errno = 0;
	rc = mprotect(addr, segment_size, PROT_NONE);
	printf("  mprotect(PROT_NONE) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
	trigger_fault_probe("  read after PROT_NONE", addr, FAULT_READ);

	errno = 0;
	rc = mprotect(addr, segment_size, PROT_READ | PROT_WRITE);
	printf("  mprotect(PROT_READ|PROT_WRITE) -> ret=%d errno=%d (%s)\n", rc,
	       errno, strerror(errno));
	addr[0] = 0x5a;
	printf("  write after restore -> byte0=0x%02x\n", addr[0]);

	shmdt(addr);
	errno = 0;
	rc = shmctl(shmid, IPC_RMID, NULL);
	printf("  shmctl(IPC_RMID) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
}

static void scenario_mremap_move_detach_accounting(void)
{
	print_case_header("mremap move and shmdt accounting");

	errno = 0;
	int shmid =
		shmget(IPC_PRIVATE, segment_size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget -> shmid=%d errno=%d (%s)\n", shmid, errno,
	       strerror(errno));
	if (shmid < 0) {
		return;
	}

	errno = 0;
	char *addr = shmat(shmid, NULL, 0);
	printf("  shmat -> addr=%p errno=%d (%s)\n", addr, errno,
	       strerror(errno));
	if (addr == (void *)-1) {
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	errno = 0;
	void *target = mmap(NULL, segment_size * 2, PROT_NONE,
			    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	printf("  mmap scratch area -> addr=%p errno=%d (%s)\n", target, errno,
	       strerror(errno));
	if (target == MAP_FAILED) {
		shmdt(addr);
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	errno = 0;
	int rc = munmap(target, segment_size * 2);
	printf("  munmap scratch -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
	if (rc < 0) {
		shmdt(addr);
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	errno = 0;
	char *moved = mremap(addr, segment_size, segment_size,
			     MREMAP_MAYMOVE | MREMAP_FIXED, target);
	printf("  mremap(MAYMOVE|FIXED) -> new addr=%p errno=%d (%s)\n", moved,
	       errno, strerror(errno));
	if (moved == MAP_FAILED) {
		shmdt(addr);
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	report_shm_nattch("  nattch after move", shmid);

	errno = 0;
	rc = shmdt(addr);
	printf("  shmdt(old pointer) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));

	errno = 0;
	rc = shmdt(moved);
	printf("  shmdt(new pointer) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));

	report_shm_nattch("  nattch after detaching new pointer", shmid);

	errno = 0;
	rc = shmctl(shmid, IPC_RMID, NULL);
	printf("  shmctl(IPC_RMID) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
}

static void scenario_key_reuse_after_rmid(void)
{
	print_case_header("key reuse after old segment marked for removal");

	key_t key = next_key();
	errno = 0;
	int old_shmid = shmget(key, segment_size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  first shmget(key=0x%lx) -> shmid=%d errno=%d (%s)\n",
	       (unsigned long)key, old_shmid, errno, strerror(errno));
	if (old_shmid < 0) {
		return;
	}

	errno = 0;
	unsigned char *old_addr = shmat(old_shmid, NULL, 0);
	printf("  first shmat -> addr=%p errno=%d (%s)\n", old_addr, errno,
	       strerror(errno));
	if (old_addr == (void *)-1) {
		shmctl(old_shmid, IPC_RMID, NULL);
		return;
	}
	old_addr[0] = 0x11;

	errno = 0;
	int rc = shmctl(old_shmid, IPC_RMID, NULL);
	printf("  shmctl(old, IPC_RMID) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));

	errno = 0;
	int new_shmid = shmget(key, segment_size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  second shmget(same key) -> shmid=%d errno=%d (%s)\n",
	       new_shmid, errno, strerror(errno));
	if (new_shmid < 0) {
		shmdt(old_addr);
		return;
	}

	errno = 0;
	unsigned char *new_addr = shmat(new_shmid, NULL, 0);
	printf("  second shmat -> addr=%p errno=%d (%s)\n", new_addr, errno,
	       strerror(errno));
	if (new_addr == (void *)-1) {
		shmdt(old_addr);
		shmctl(new_shmid, IPC_RMID, NULL);
		return;
	}
	new_addr[0] = 0x22;

	printf("  byte in old mapping = 0x%02x, new mapping = 0x%02x\n",
	       old_addr[0], new_addr[0]);

	shmdt(old_addr);
	shmdt(new_addr);

	errno = 0;
	rc = shmctl(new_shmid, IPC_RMID, NULL);
	printf("  shmctl(new, IPC_RMID) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
}

static void scenario_concurrent_faults_and_final_detach(void)
{
	print_case_header("concurrent faults + last detach cleanup");

	errno = 0;
	int shmid =
		shmget(IPC_PRIVATE, segment_size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget -> shmid=%d errno=%d (%s)\n", shmid, errno,
	       strerror(errno));
	if (shmid < 0) {
		return;
	}

	errno = 0;
	unsigned char *addr = shmat(shmid, NULL, 0);
	printf("  shmat -> addr=%p errno=%d (%s)\n", addr, errno,
	       strerror(errno));
	if (addr == (void *)-1) {
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	errno = 0;
	int rc = shmctl(shmid, IPC_RMID, NULL);
	printf("  shmctl(IPC_RMID) before faults -> ret=%d errno=%d (%s)\n", rc,
	       errno, strerror(errno));

	pid_t pid = fork();
	if (pid < 0) {
		perror("fork");
		shmdt(addr);
		return;
	}

	if (pid == 0) {
		dirty_all_pages(addr);
		printf("  [child] dirtied pages and exits\n");
		_exit(0);
	}

	dirty_all_pages(addr);
	printf("  [parent] dirtied pages locally\n");

	int status = 0;
	waitpid(pid, &status, 0);
	printf("  waitpid -> status=0x%x\n", status);

	report_shm_nattch("  nattch before final detach", shmid);

	errno = 0;
	rc = shmdt(addr);
	printf("  shmdt -> ret=%d errno=%d (%s)\n", rc, errno, strerror(errno));

	struct shmid_ds ds;
	errno = 0;
	rc = shmctl(shmid, IPC_STAT, &ds);
	printf("  shmctl(IPC_STAT) after last detach -> ret=%d errno=%d (%s)\n",
	       rc, errno, strerror(errno));
}

static void scenario_shmat_fixed_address_conflict(void)
{
	print_case_header("shmat with fixed address conflicting VMA");

	errno = 0;
	int shmid =
		shmget(IPC_PRIVATE, segment_size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget -> shmid=%d errno=%d (%s)\n", shmid, errno,
	       strerror(errno));
	if (shmid < 0) {
		return;
	}

	errno = 0;
	void *occupied = mmap(NULL, segment_size, PROT_READ | PROT_WRITE,
			      MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	printf("  mmap existing VMA -> addr=%p errno=%d (%s)\n", occupied,
	       errno, strerror(errno));
	if (occupied == MAP_FAILED) {
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	errno = 0;
	void *addr = shmat(shmid, occupied, 0);
	printf("  shmat(conflict address) -> addr=%p errno=%d (%s)\n", addr,
	       errno, strerror(errno));

	munmap(occupied, segment_size);

	errno = 0;
	void *auto_addr = shmat(shmid, NULL, 0);
	printf("  shmat(NULL) -> addr=%p errno=%d (%s)\n", auto_addr, errno,
	       strerror(errno));
	if (auto_addr != (void *)-1) {
		shmdt(auto_addr);
	}

	errno = 0;
	int rc = shmctl(shmid, IPC_RMID, NULL);
	printf("  shmctl(IPC_RMID) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
}

static void scenario_nonpage_aligned_address(void)
{
	print_case_header("shmat with non page-aligned address");

	errno = 0;
	int shmid =
		shmget(IPC_PRIVATE, segment_size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget -> shmid=%d errno=%d (%s)\n", shmid, errno,
	       strerror(errno));
	if (shmid < 0) {
		return;
	}

	errno = 0;
	char *tmp = mmap(NULL, segment_size * 2, PROT_READ | PROT_WRITE,
			 MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	printf("  mmap scratch -> addr=%p errno=%d (%s)\n", tmp, errno,
	       strerror(errno));
	if (tmp == MAP_FAILED) {
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	void *unaligned = (void *)(tmp + 1);
	errno = 0;
	void *addr = shmat(shmid, unaligned, 0);
	printf("  shmat(unaligned addr=%p) -> addr=%p errno=%d (%s)\n",
	       unaligned, addr, errno, strerror(errno));

	munmap(tmp, segment_size * 2);

	errno = 0;
	int rc = shmctl(shmid, IPC_RMID, NULL);
	printf("  shmctl(IPC_RMID) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
}

static void scenario_partial_detach(void)
{
	print_case_header("shmdt with non-start address");

	errno = 0;
	int shmid =
		shmget(IPC_PRIVATE, segment_size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget -> shmid=%d errno=%d (%s)\n", shmid, errno,
	       strerror(errno));
	if (shmid < 0) {
		return;
	}

	errno = 0;
	unsigned char *addr = shmat(shmid, NULL, 0);
	printf("  shmat -> addr=%p errno=%d (%s)\n", addr, errno,
	       strerror(errno));
	if (addr == (void *)-1) {
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	errno = 0;
	int rc = shmdt(addr + page_size);
	printf("  shmdt(addr + page_size) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));

	errno = 0;
	rc = shmdt(addr);
	printf("  shmdt(base addr) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));

	errno = 0;
	rc = shmctl(shmid, IPC_RMID, NULL);
	printf("  shmctl(IPC_RMID) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
}

static void scenario_munmap_vs_shmdt(void)
{
	print_case_header("munmap vs shmdt consistency");

	errno = 0;
	int shmid =
		shmget(IPC_PRIVATE, segment_size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget -> shmid=%d errno=%d (%s)\n", shmid, errno,
	       strerror(errno));
	if (shmid < 0) {
		return;
	}

	errno = 0;
	char *addr = shmat(shmid, NULL, 0);
	printf("  shmat -> addr=%p errno=%d (%s)\n", addr, errno,
	       strerror(errno));
	if (addr == (void *)-1) {
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	report_shm_nattch("  nattch before munmap", shmid);

	errno = 0;
	int rc = munmap(addr, segment_size);
	printf("  munmap -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));

	report_shm_nattch("  nattch after munmap", shmid);

	errno = 0;
	rc = shmdt(addr);
	printf("  shmdt after munmap -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));

	errno = 0;
	rc = shmctl(shmid, IPC_RMID, NULL);
	printf("  shmctl(IPC_RMID) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
}

static void scenario_partial_munmap_attachment_behavior(void)
{
	print_case_header("partial munmap keeps attachment");

	errno = 0;
	int shmid =
		shmget(IPC_PRIVATE, segment_size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget -> shmid=%d errno=%d (%s)\n", shmid, errno,
	       strerror(errno));
	if (shmid < 0) {
		return;
	}

	errno = 0;
	unsigned char *addr = shmat(shmid, NULL, 0);
	printf("  shmat -> addr=%p errno=%d (%s)\n", addr, errno,
	       strerror(errno));
	if (addr == (void *)-1) {
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	dirty_all_pages(addr);
	report_shm_nattch("  nattch before partial munmap", shmid);

	errno = 0;
	int rc = munmap(addr + page_size, page_size);
	printf("  munmap(middle page) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));

	report_shm_nattch("  nattch after partial munmap", shmid);

	trigger_fault_probe("  probe unmapped gap",
			    (volatile char *)(addr + page_size), FAULT_READ);
	trigger_fault_probe("  probe first page", (volatile char *)addr,
			    FAULT_READ);
	trigger_fault_probe(
		"  probe last page",
		(volatile char *)(addr +
				  page_size * (segment_size / page_size - 1)),
		FAULT_READ);

	errno = 0;
	rc = shmdt(addr + 2 * page_size);
	printf("  shmdt(base addr + 2 * page_size) -> ret=%d errno=%d (%s)\n",
	       rc, errno, strerror(errno));

	errno = 0;
	rc = shmdt(addr);
	printf("  shmdt(base addr after partial munmap) -> ret=%d errno=%d (%s)\n",
	       rc, errno, strerror(errno));

	report_shm_nattch("  nattch after shmdt", shmid);

	errno = 0;
	rc = shmctl(shmid, IPC_RMID, NULL);
	printf("  shmctl(IPC_RMID) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
}

static void scenario_partial_munmap_refcnt_split(void)
{
	print_case_header("partial munmap split keeps nattch stable");

	const size_t size = page_size * 3;
	errno = 0;
	int shmid = shmget(IPC_PRIVATE, size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget(size=%zu) -> shmid=%d errno=%d (%s)\n", size, shmid,
	       errno, strerror(errno));
	if (shmid < 0) {
		exit(EXIT_FAILURE);
	}

	errno = 0;
	unsigned char *addr = shmat(shmid, NULL, 0);
	printf("  shmat -> addr=%p errno=%d (%s)\n", addr, errno,
	       strerror(errno));
	if (addr == (void *)-1) {
		exit(EXIT_FAILURE);
	}

	expect_shm_nattch_eq("  after shmat", shmid, 1);

	unsigned char *middle = addr + page_size;
	errno = 0;
	int rc = munmap(middle, page_size);
	printf("  munmap(middle page) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
	if (rc < 0) {
		exit(EXIT_FAILURE);
	}

	expect_shm_nattch_eq("  after munmap middle", shmid, 2);

	int signo = probe_access((volatile char *)middle, FAULT_READ);
	if (signo == 0) {
		fprintf(stderr, "expected fault when reading middle page\n");
		exit(EXIT_FAILURE);
	}
	if (probe_access((volatile char *)addr, FAULT_READ) != 0) {
		fprintf(stderr, "unexpected fault when reading first page\n");
		exit(EXIT_FAILURE);
	}
	if (probe_access((volatile char *)(addr + page_size * 2), FAULT_READ) !=
	    0) {
		fprintf(stderr, "unexpected fault when reading last page\n");
		exit(EXIT_FAILURE);
	}

	errno = 0;
	rc = shmdt(addr);
	printf("  shmdt(base addr) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
	if (rc < 0) {
		exit(EXIT_FAILURE);
	}

	expect_shm_nattch_eq("  after shmdt", shmid, 0);

	if (probe_access((volatile char *)addr, FAULT_READ) == 0) {
		fprintf(stderr, "expected fault after shmdt on first page\n");
		exit(EXIT_FAILURE);
	}
	if (probe_access((volatile char *)(addr + page_size * 2), FAULT_READ) ==
	    0) {
		fprintf(stderr, "expected fault after shmdt on last page\n");
		exit(EXIT_FAILURE);
	}

	errno = 0;
	rc = shmctl(shmid, IPC_RMID, NULL);
	printf("  shmctl(IPC_RMID) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
	if (rc < 0) {
		exit(EXIT_FAILURE);
	}
}

static void scenario_mremap_shrink_and_expand(void)
{
	print_case_header("mremap shrink and expand behavior");

	errno = 0;
	int shmid =
		shmget(IPC_PRIVATE, segment_size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget -> shmid=%d errno=%d (%s)\n", shmid, errno,
	       strerror(errno));
	if (shmid < 0) {
		return;
	}

	errno = 0;
	char *addr = shmat(shmid, NULL, 0);
	printf("  shmat -> addr=%p errno=%d (%s)\n", addr, errno,
	       strerror(errno));
	if (addr == (void *)-1) {
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	errno = 0;
	char *shrunk = mremap(addr, segment_size, segment_size / 2, 0);
	printf("  mremap shrink -> addr=%p errno=%d (%s)\n", shrunk, errno,
	       strerror(errno));
	if (shrunk == MAP_FAILED) {
		shmdt(addr);
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	trigger_fault_probe("  probe beyond new size",
			    shrunk + segment_size / 2, FAULT_READ);

	errno = 0;
	char *expanded = mremap(shrunk, segment_size / 2, segment_size * 2,
				MREMAP_MAYMOVE);
	printf("  mremap expand -> addr=%p errno=%d (%s)\n", expanded, errno,
	       strerror(errno));
	if (expanded == MAP_FAILED) {
		shmdt(shrunk);
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	trigger_fault_probe("  write within expanded area",
			    expanded + segment_size + page_size, FAULT_WRITE);

	trigger_fault_probe("  probe beyond expanded size",
			    expanded + segment_size * 2, FAULT_WRITE);

	shmdt(expanded);
	errno = 0;
	int rc = shmctl(shmid, IPC_RMID, NULL);
	printf("  shmctl(IPC_RMID) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
}

static void scenario_shmget_size_increase_einval(void)
{
	print_case_header("shmget larger size on existing key returns EINVAL");

	key_t key = next_key();
	errno = 0;
	int shmid = shmget(key, page_size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget(size=%zu) -> shmid=%d errno=%d (%s)\n", page_size,
	       shmid, errno, strerror(errno));
	if (shmid < 0) {
		return;
	}

	struct shmid_ds ds;
	errno = 0;
	int rc = shmctl(shmid, IPC_STAT, &ds);
	printf("  shmctl(IPC_STAT) -> ret=%d errno=%d (%s) size=%zu\n", rc,
	       errno, strerror(errno), (size_t)ds.shm_segsz);

	errno = 0;
	int second = shmget(key, page_size * 2, IPC_CREAT | 0600);
	printf("  shmget(size=%zu) again -> shmid=%d errno=%d (%s)\n",
	       page_size * 2, second, errno, strerror(errno));

	errno = 0;
	rc = shmctl(shmid, IPC_RMID, NULL);
	printf("  shmctl(IPC_RMID) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
}

static void scenario_middle_page_replaced_by_anonymous(void)
{
	print_case_header(
		"middle page replaced by anonymous mapping survives shmdt");

	size_t size = page_size * 3;
	errno = 0;
	int shmid = shmget(IPC_PRIVATE, size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget(size=%zu) -> shmid=%d errno=%d (%s)\n", size, shmid,
	       errno, strerror(errno));
	if (shmid < 0) {
		return;
	}

	errno = 0;
	unsigned char *addr = shmat(shmid, NULL, 0);
	printf("  shmat -> addr=%p errno=%d (%s)\n", addr, errno,
	       strerror(errno));
	if (addr == (void *)-1) {
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	unsigned char *middle = addr + page_size;
	errno = 0;
	int rc = munmap(middle, page_size);
	printf("  munmap(middle page) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
	if (rc < 0) {
		shmdt(addr);
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	errno = 0;
	void *fixed = mmap(middle, page_size, PROT_READ | PROT_WRITE,
			   MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
	printf("  mmap(MAP_FIXED) -> addr=%p errno=%d (%s)\n", fixed, errno,
	       strerror(errno));
	if (fixed == MAP_FAILED) {
		shmdt(addr);
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	middle[0] = 'A';
	printf("  wrote 'A' into anonymous middle page\n");

	errno = 0;
	rc = shmdt(addr);
	printf("  shmdt -> ret=%d errno=%d (%s)\n", rc, errno, strerror(errno));

	probe_read_value("  read anonymous middle page after shmdt", middle);

	munmap(middle, page_size);
	shmctl(shmid, IPC_RMID, NULL);
}

static void scenario_unmap_first_page_then_shmdt(void)
{
	print_case_header("unmap first page then shmdt removes remaining page");

	size_t size = page_size * 2;
	errno = 0;
	int shmid = shmget(IPC_PRIVATE, size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget(size=%zu) -> shmid=%d errno=%d (%s)\n", size, shmid,
	       errno, strerror(errno));
	if (shmid < 0) {
		return;
	}

	errno = 0;
	unsigned char *addr = shmat(shmid, NULL, 0);
	printf("  shmat -> addr=%p errno=%d (%s)\n", addr, errno,
	       strerror(errno));
	if (addr == (void *)-1) {
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	unsigned char *second = addr + page_size;
	second[0] = 'A';
	printf("  wrote 'A' to second page\n");

	errno = 0;
	int rc = munmap(addr, page_size);
	printf("  munmap(first page) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
	if (rc < 0) {
		shmdt(addr);
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	probe_read_value("  read second page before shmdt", second);

	errno = 0;
	rc = shmdt(addr);
	printf("  shmdt(base addr) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));

	trigger_fault_probe("  read second page after shmdt",
			    (volatile char *)second, FAULT_READ);

	shmctl(shmid, IPC_RMID, NULL);
}

static void scenario_detach_higher_mapping_keeps_lower(void)
{
	print_case_header("detaching higher shmat keeps lower mapping");

	size_t size = page_size * 2;
	errno = 0;
	int shmid = shmget(IPC_PRIVATE, size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget(size=%zu) -> shmid=%d errno=%d (%s)\n", size, shmid,
	       errno, strerror(errno));
	if (shmid < 0) {
		return;
	}

	errno = 0;
	unsigned char *addr1 = shmat(shmid, NULL, 0);
	printf("  shmat #1 -> addr=%p errno=%d (%s)\n", addr1, errno,
	       strerror(errno));
	if (addr1 == (void *)-1) {
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	errno = 0;
	unsigned char *addr2 = shmat(shmid, NULL, 0);
	printf("  shmat #2 -> addr=%p errno=%d (%s)\n", addr2, errno,
	       strerror(errno));
	if (addr2 == (void *)-1) {
		shmdt(addr1);
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	unsigned char *higher = addr1 > addr2 ? addr1 : addr2;
	unsigned char *lower = addr1 > addr2 ? addr2 : addr1;

	errno = 0;
	int rc = shmdt(higher);
	printf("  shmdt(higher=%p) -> ret=%d errno=%d (%s)\n", higher, rc,
	       errno, strerror(errno));

	lower[0] = 0x55;
	printf("  wrote to lower mapping after shmdt\n");
	probe_read_value("  read from lower mapping", lower);

	shmdt(lower);
	shmctl(shmid, IPC_RMID, NULL);
}

static void scenario_mremap_move_second_page(void)
{
	print_case_header("mremap moved page requires separate shmdt");

	size_t size = page_size * 2;
	errno = 0;
	int shmid = shmget(IPC_PRIVATE, size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget(size=%zu) -> shmid=%d errno=%d (%s)\n", size, shmid,
	       errno, strerror(errno));
	if (shmid < 0) {
		return;
	}

	errno = 0;
	unsigned char *addr = shmat(shmid, NULL, 0);
	printf("  shmat -> addr=%p errno=%d (%s)\n", addr, errno,
	       strerror(errno));
	if (addr == (void *)-1) {
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	addr[0] = 0x11;
	addr[page_size] = 0x22;

	errno = 0;
	void *target = mmap(NULL, page_size, PROT_READ | PROT_WRITE,
			    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	printf("  mmap target -> addr=%p errno=%d (%s)\n", target, errno,
	       strerror(errno));
	if (target == MAP_FAILED) {
		shmdt(addr);
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	errno = 0;
	void *moved = mremap(addr + page_size, page_size, page_size,
			     MREMAP_MAYMOVE | MREMAP_FIXED, target);
	printf("  mremap(second page) -> addr=%p errno=%d (%s)\n", moved, errno,
	       strerror(errno));
	if (moved == MAP_FAILED) {
		shmdt(addr);
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	report_shm_nattch("  nattch after mremap", shmid);

	errno = 0;
	int rc = shmdt(addr);
	printf("  shmdt(original base) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));

	probe_read_value("  read moved page after shmdt(base)",
			 (unsigned char *)moved);

	errno = 0;
	rc = shmdt((char *)moved - page_size);
	printf("  shmdt(moved - PAGE_SIZE) -> ret=%d errno=%d (%s)\n", rc,
	       errno, strerror(errno));

	trigger_fault_probe("  read moved page after shmdt(moved - PAGE_SIZE)",
			    (volatile char *)moved, FAULT_READ);

	shmctl(shmid, IPC_RMID, NULL);
}

static void scenario_cross_attach_mremap_double_shmdt(void)
{
	print_case_header("cross-attach remap needs two shmdt calls");

	size_t size = page_size * 2;
	errno = 0;
	int shmid = shmget(IPC_PRIVATE, size, IPC_CREAT | IPC_EXCL | 0600);
	printf("  shmget(size=%zu) -> shmid=%d errno=%d (%s)\n", size, shmid,
	       errno, strerror(errno));
	if (shmid < 0) {
		return;
	}

	report_shm_nattch("  after shmget", shmid);

	errno = 0;
	unsigned char *addr1 = shmat(shmid, NULL, 0);
	printf("  shmat addr1 -> addr=%p errno=%d (%s)\n", addr1, errno,
	       strerror(errno));
	if (addr1 == (void *)-1) {
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	errno = 0;
	unsigned char *addr2 = shmat(shmid, NULL, 0);
	printf("  shmat addr2 -> addr=%p errno=%d (%s)\n", addr2, errno,
	       strerror(errno));
	if (addr2 == (void *)-1) {
		shmdt(addr1);
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}

	memset(addr1, 0x11, size);
	memset(addr2, 0x22, size);

	printf("  munmap(addr2 + PAGE_SIZE)\n");
	errno = 0;
	int rc = munmap(addr2 + page_size, page_size);
	printf("  munmap -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
	report_shm_nattch("  after munmap addr2 second page", shmid);

	printf("  munmap(addr1)\n");
	errno = 0;
	rc = munmap(addr1, page_size);
	printf("  munmap -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
	report_shm_nattch("  after munmap addr1 first page", shmid);

	void *src = addr1 + page_size;
	void *dst = addr2 + page_size;
	printf("  mremap addr1 second page -> addr2 second page\n");
	printf("  src=%p dst=%p\n", src, dst);
	errno = 0;
	void *r = mremap(src, page_size, page_size,
			 MREMAP_MAYMOVE | MREMAP_FIXED, dst);
	printf("  mremap -> addr=%p errno=%d (%s)\n", r, errno,
	       strerror(errno));
	if (r == MAP_FAILED) {
		shmdt(addr1);
		shmdt(addr2);
		shmctl(shmid, IPC_RMID, NULL);
		return;
	}
	report_shm_nattch("  after mremap", shmid);

	printf("  shmdt(addr2)\n");
	errno = 0;
	rc = shmdt(addr2);
	printf("  shmdt(addr2) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
	report_shm_nattch("  after shmdt addr2", shmid);

	printf("  shmdt(addr2) again\n");
	errno = 0;
	rc = shmdt(addr2);
	printf("  shmdt(addr2) -> ret=%d errno=%d (%s)\n", rc, errno,
	       strerror(errno));
	report_shm_nattch("  after shmdt addr2 again", shmid);

	shmctl(shmid, IPC_RMID, NULL);
}

int main(void)
{
	page_size = (size_t)sysconf(_SC_PAGESIZE);
	if (page_size == 0) {
		fprintf(stderr, "sysconf(_SC_PAGESIZE) failed\n");
		return 1;
	}
	segment_size = page_size * 4;

	setvbuf(stdout, NULL, _IONBF, 0);

	printf("page_size=%zu bytes, test segment size=%zu bytes\n", page_size,
	       segment_size);
	install_fault_handlers();

	scenario_ipc_rmid_existing_mapping();
	scenario_fork_shared_writes();
	scenario_fork_refcnt_multi_attach();
	scenario_readonly_attach_no_upgrade();
	scenario_mprotect_downgrade_faults();
	scenario_mremap_move_detach_accounting();
	scenario_key_reuse_after_rmid();
	scenario_concurrent_faults_and_final_detach();
	scenario_shmat_fixed_address_conflict();
	scenario_nonpage_aligned_address();
	scenario_partial_detach();
	scenario_munmap_vs_shmdt();
	scenario_partial_munmap_attachment_behavior();
	scenario_partial_munmap_refcnt_split();
	scenario_mremap_shrink_and_expand();
	scenario_shmget_size_increase_einval();
	scenario_middle_page_replaced_by_anonymous();
	scenario_unmap_first_page_then_shmdt();
	scenario_detach_higher_mapping_keeps_lower();
	scenario_mremap_move_second_page();
	scenario_cross_attach_mremap_double_shmdt();

	return 0;
}
