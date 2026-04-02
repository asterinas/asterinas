// SPDX-License-Identifier: MPL-2.0

#include <sys/prctl.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/wait.h>
#include <errno.h>

void set_child_subreaper()
{
	if (prctl(PR_SET_CHILD_SUBREAPER, 1) == -1) {
		perror("prctl: PR_SET_CHILD_SUBREAPER failed");
		exit(EXIT_FAILURE);
	}
	printf("Process %d is now a child subreaper\n", getpid());
}

void get_child_subreaper()
{
	int subreaper;
	if (prctl(PR_GET_CHILD_SUBREAPER, &subreaper) == -1) {
		perror("prctl: PR_GET_CHILD_SUBREAPER failed");
		exit(EXIT_FAILURE);
	}
	printf("Process %d has_child_subreaper: %d\n", getpid(), subreaper);
}

void print_process_info(const char *name)
{
	printf("%s: PID=%d, PPID=%d\n", name, getpid(), getppid());
}

void child_process()
{
	print_process_info("Child process");

	pid_t grandchild_pid = fork();
	if (grandchild_pid < 0) {
		perror("fork failed");
		exit(EXIT_FAILURE);
	} else if (grandchild_pid == 0) {
		print_process_info("Grandchild process");
		sleep(2);
		printf("Grandchild process %d exiting\n", getpid());
		exit(EXIT_SUCCESS);
	} else {
		sleep(1);
		printf("Child process %d exiting\n", getpid());
		exit(EXIT_SUCCESS);
	}
}

int main()
{
	// Set the current process as the subreaper.
	set_child_subreaper();
	get_child_subreaper();

	pid_t child_pid = fork();
	if (child_pid < 0) {
		perror("fork failed");
		exit(EXIT_FAILURE);
	} else if (child_pid == 0) {
		child_process();
	} else {
		print_process_info("Parent process");

		// Wait for the son process to exit
		waitpid(child_pid, NULL, 0);
		printf("Parent process %d: child %d exited\n", getpid(),
		       child_pid);

		// Wait for the grandson process to exit
		printf("Parent process %d waiting for grandchild\n", getpid());

		int status;
		pid_t waited_pid = wait(&status);
		// The first time successfully waited for the grandchild process to exit.
		if (waited_pid != -1) {
			printf("Parent process %d: grandchild %d exited with status %d\n",
			       getpid(), waited_pid, WEXITSTATUS(status));
		} else {
			exit(EXIT_FAILURE);
		}

		waited_pid = wait(&status);
		// The second time there were no processes left to wait for.
		if (waited_pid == -1) {
			if (errno == ECHILD) {
				printf("Parent process %d: no more children to wait for\n",
				       getpid());
			} else {
				perror("wait failed");
				exit(EXIT_FAILURE);
			}
		}
	}

	return 0;
}