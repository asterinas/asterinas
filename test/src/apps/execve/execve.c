// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <unistd.h>

int main()
{
	char *argv[] = { "argv1", "argv2", NULL };
	char *envp[] = { "home=/", "version=1.1", NULL };
	// The hello will be put at /execve/hello in InitRamfs
	printf("Execve a new file /execve/hello:\n");
	// flush the stdout content to ensure the content print to console
	fflush(stdout);
	execve("/test/execve/hello", argv, envp);
	printf("Should not print\n");
	fflush(stdout);
	return 0;
}
