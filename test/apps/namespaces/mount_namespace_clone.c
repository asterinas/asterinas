// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mount.h>
#include <unistd.h>
#include <sys/wait.h>
#include <errno.h>
#include <dirent.h>
#include <sys/stat.h>

// Function to list contents of a directory.
void list_dir(const char *path)
{
	DIR *d;
	struct dirent *dir;
	d = opendir(path);
	if (d) {
		printf("------Contents of %s:\n", path);
		while ((dir = readdir(d)) != NULL) {
			printf("%s\n", dir->d_name);
		}
		closedir(d);
	} else {
		perror("opendir");
	}
}

int child_fn(void *arg)
{
	printf("--------In child process---------\n");

	// Mount a new ext2 filesystem.
	if (mount("vext2", "/mnt", "ext2", 0, "") != 0) {
		perror("mount");
		return 1;
	}

	printf("Mounted ext2 on /mnt in new namespace\n");

	// Check the mount status in the new namespace.
	list_dir("/mnt");

	// Unmount the old ext2 filesystem.
	if (umount("/ext2") != 0) {
		perror("umount");
		return 1;
	}

	printf("Unmounted /ext2 in new namespace\n");

	// Check the mount status again after unmounting.
	list_dir("/ext2");

	return 0;
}

#define STACK_SIZE (1024 * 1024) // Define the stack size for the child process.

int main()
{
	printf("--------In parent process---------\n");

	// Create the /mnt directory if it doesn't exist
	if (mkdir("/mnt", 0755) == -1 && errno != EEXIST) {
		perror("mkdir");
		exit(EXIT_FAILURE);
	}

	// Check the initial mount status in the old namespace.
	printf("Check old namespace mounts:\n");
	list_dir("/mnt");
	list_dir("/ext2");

	// Allocate stack for the child process.
	char *stack = malloc(STACK_SIZE);
	if (stack == NULL) {
		perror("malloc");
		exit(EXIT_FAILURE);
	}

	// Create a new child process using clone.
	pid_t pid = clone(child_fn, stack + STACK_SIZE, CLONE_NEWNS | SIGCHLD,
			  NULL);
	if (pid == -1) {
		perror("clone");
		free(stack);
		exit(EXIT_FAILURE);
	}

	// In parent process, wait for the child to finish.
	if (waitpid(pid, NULL, 0) == -1) {
		perror("waitpid");
		free(stack);
		exit(EXIT_FAILURE);
	}
	printf("--------Child process exited--------\n");

	// Free the allocated stack.
	free(stack);

	// Check mounts in the parent namespace after child exits.
	printf("Check old namespace mounts:\n");
	list_dir("/mnt");
	list_dir("/ext2");

	return 0;
}
