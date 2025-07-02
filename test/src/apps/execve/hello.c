// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>

int main(int argc, char *argv[], char *envp[])
{
	printf("Hello world from hello.c(execved in execve.c)!\n");
	printf("argc = %d\n", argc);
	for (int i = 0; i < argc; i++) {
		printf("%s\n", argv[i]);
	}
	for (int i = 0;; i++) {
		if (envp[i] == NULL) {
			break;
		}
		printf("%s\n", envp[i]);
	}
	return 0;
}
