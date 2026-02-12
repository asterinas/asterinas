// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <unistd.h>

int main()
{
	printf("before fork\n");
	fflush(stdout);
	if (fork() == 0) {
		printf("after fork: Hello from child\n");
	} else {
		printf("after fork: Hello from parent\n");
	}
	fflush(stdout);
	return 0;
}
