#define _GNU_SOURCE
#include <sys/ptrace.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <sys/user.h>
#include <signal.h>
#include <unistd.h>
#include <stdio.h>
#include <stdlib.h>
#include <errno.h>
#include <string.h>

static void dump_proc_status(pid_t pid)
{
	char path[64];
	snprintf(path, sizeof(path), "/proc/%d/status", pid);

	FILE *f = fopen(path, "r");
	if (!f) {
		perror("fopen(/proc/pid/status)");
		return;
	}

	printf("----- /proc/%d/status -----\n", pid);

	char buf[256];
	while (fgets(buf, sizeof(buf), f)) {
		fputs(buf, stdout);
	}

	printf("----- end status -----\n");
	fclose(f);
}

static void print_waitid_siginfo(const siginfo_t *si)
{
	printf("waitid: si_code=%d ", si->si_code);

	switch (si->si_code) {
	case CLD_EXITED:
		printf("(CLD_EXITED)\n");
		printf("  child exited, status=%d\n", si->si_status);
		break;

	case CLD_KILLED:
		printf("(CLD_KILLED)\n");
		printf("  child killed by signal %d (%s)\n", si->si_status,
		       strsignal(si->si_status));
		break;

	case CLD_DUMPED:
		printf("(CLD_DUMPED)\n");
		printf("  child dumped core by signal %d (%s)\n", si->si_status,
		       strsignal(si->si_status));
		break;

	case CLD_STOPPED:
		printf("(CLD_STOPPED)\n");
		printf("  child stopped by signal %d (%s)\n", si->si_status,
		       strsignal(si->si_status));
		break;

	case CLD_TRAPPED:
		printf("(CLD_TRAPPED)\n");
		printf("  child trapped by signal %d (%s)\n", si->si_status,
		       strsignal(si->si_status));
		break;

	case CLD_CONTINUED:
		printf("(CLD_CONTINUED)\n");
		printf("  child continued\n");
		break;

	default:
		printf("(unknown)\n");
		printf("  si_status=%d\n", si->si_status);
		break;
	}
}

static void print_regs(const struct user_regs_struct *r)
{
	printf("r15=0x%016llx r14=0x%016llx r13=0x%016llx r12=0x%016llx\n",
	       r->r15, r->r14, r->r13, r->r12);

	printf("rbp=0x%016llx rbx=0x%016llx r11=0x%016llx r10=0x%016llx\n",
	       r->rbp, r->rbx, r->r11, r->r10);

	printf("r9 =0x%016llx r8 =0x%016llx rax=0x%016llx rcx=0x%016llx\n",
	       r->r9, r->r8, r->rax, r->rcx);

	printf("rdx=0x%016llx rsi=0x%016llx rdi=0x%016llx\n", r->rdx, r->rsi,
	       r->rdi);

	printf("orig_rax=0x%016llx rip=0x%016llx\n", r->orig_rax, r->rip);

	printf("cs=0x%016llx eflags=0x%016llx\n", r->cs, r->eflags);

	printf("rsp=0x%016llx ss=0x%016llx\n", r->rsp, r->ss);

	printf("fs_base=0x%016llx gs_base=0x%016llx\n", r->fs_base, r->gs_base);

	printf("ds=0x%016llx es=0x%016llx fs=0x%016llx gs=0x%016llx\n", r->ds,
	       r->es, r->fs, r->gs);
}

static void ptrace_regs_roundtrip(pid_t pid, const char *stage)
{
	struct user_regs_struct before, after_set, after_get, restored_get;

	if (ptrace(PTRACE_GETREGS, pid, NULL, &before) == -1) {
		perror("ptrace(GETREGS before)");
		exit(1);
	}

	printf("[parent][%s] regs before set:\n", stage);
	print_regs(&before);

	after_set = before;
	after_set.r15 ^= 0x5a5a5a5a5a5a5a5aULL;

	if (ptrace(PTRACE_SETREGS, pid, NULL, &after_set) == -1) {
		perror("ptrace(SETREGS)");
		exit(1);
	}

	if (ptrace(PTRACE_GETREGS, pid, NULL, &after_get) == -1) {
		perror("ptrace(GETREGS after set)");
		exit(1);
	}

	printf("[parent][%s] regs after set:\n", stage);
	print_regs(&after_get);

	if (after_get.r15 != after_set.r15) {
		fprintf(stderr,
			"[parent][%s] SETREGS not visible via GETREGS: expected r15=0x%016llx got r15=0x%016llx\n",
			stage, after_set.r15, after_get.r15);
		exit(1);
	}

	if (ptrace(PTRACE_SETREGS, pid, NULL, &before) == -1) {
		perror("ptrace(SETREGS restore)");
		exit(1);
	}

	if (ptrace(PTRACE_GETREGS, pid, NULL, &restored_get) == -1) {
		perror("ptrace(GETREGS after restore)");
		exit(1);
	}

	if (restored_get.r15 != before.r15) {
		fprintf(stderr,
			"[parent][%s] restore failed: expected r15=0x%016llx got r15=0x%016llx\n",
			stage, before.r15, restored_get.r15);
		exit(1);
	}
}

int main(void)
{
	pid_t pid = fork();
	if (pid < 0) {
		perror("fork");
		exit(1);
	}

	if (pid == 0) {
		/* Child */
		if (ptrace(PTRACE_TRACEME, 0, NULL, NULL) == -1) {
			perror("ptrace(TRACEME)");
			exit(1);
		}

		printf("[child] pid=%d, TRACEME set\n", getpid());
		fflush(stdout);
		char *const argv[] = { "/test/process/ptrace/test2", NULL };
		char *const envp[] = { NULL };

		execve(argv[0], argv, envp);
		perror("execve(test2)");
		exit(1);
	} else {
		/* Parent */
		printf("[parent] pid=%d, child pid=%d\n", getpid(), pid);
		fflush(stdout);

		sleep(1);

		/* ★ 新增：打印子进程 /proc/pid/status */
		dump_proc_status(pid);

		/* ★ 用 waitid 观察 ptrace-stop */
		siginfo_t si;
		memset(&si, 0, sizeof(si));

		if (waitid(P_PID, pid, &si, WSTOPPED) == -1) {
			perror("waitid");
			exit(1);
		}

		print_waitid_siginfo(&si);
		ptrace_regs_roundtrip(pid, "first-stop");
		if (si.si_code != CLD_TRAPPED || si.si_status != SIGTRAP) {
			fprintf(stderr,
				"[parent] unexpected first ptrace stop: code=%d status=%d\n",
				si.si_code, si.si_status);
			exit(1);
		}

		/* ★ 新增：打印子进程 /proc/pid/status */
		dump_proc_status(pid);

		ptrace(PTRACE_CONT, pid, NULL, NULL);

		memset(&si, 0, sizeof(si));
		if (waitid(P_PID, pid, &si, WSTOPPED) == -1) {
			perror("waitid(second)");
			exit(1);
		}

		print_waitid_siginfo(&si);
		ptrace_regs_roundtrip(pid, "second-stop");
		if (si.si_code != CLD_TRAPPED || si.si_status != SIGCHLD) {
			fprintf(stderr,
				"[parent] unexpected second ptrace stop: code=%d status=%d\n",
				si.si_code, si.si_status);
			exit(1);
		}

		ptrace(PTRACE_CONT, pid, NULL, NULL);

		/* ★ 用 waitpid 观察继续执行后的状态变化 */
		int status;
		waitpid(pid, &status, 0);
		if (WIFEXITED(status)) {
			printf("[parent] child exited with status %d\n",
			       WEXITSTATUS(status));
		}

		printf("[parent] exiting\n");
		return 0;
	}
}
