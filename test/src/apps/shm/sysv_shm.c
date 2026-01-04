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
	scenario_mremap_shrink_and_expand();

	return 0;
}
