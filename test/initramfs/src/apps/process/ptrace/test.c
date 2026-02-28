#define _GNU_SOURCE
#include <sys/ptrace.h>
#include <sys/types.h>
#include <sys/wait.h>
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

		for (int i = 1; i <= 3; i++) {
			sleep(1);
			printf("[child] alive\n");
			fflush(stdout);
		}

		raise(SIGSTOP);

		for (int i = 1; i <= 3; i++) {
			sleep(1);
			printf("[child] alive after stop\n");
			fflush(stdout);
		}

		exit(3);
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

		/* ★ 新增：打印子进程 /proc/pid/status */
		dump_proc_status(pid);

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