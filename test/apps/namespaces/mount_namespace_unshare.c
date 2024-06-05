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

int child_fn()
{
	printf("--------In child process---------\n");

	// Use unshare to create new mount namespace.
	if (unshare(CLONE_NEWNS) == -1) {
		perror("unshare");
		return 1;
	}

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

	// Create a new child process.
	pid_t pid = fork();
	if (pid == -1) {
		perror("fork");
		exit(EXIT_FAILURE);
	}

	if (pid == 0) {
		// In child process run the child function.
		exit(child_fn());
	} else {
		// In parent process, wait for the child to finish.
		if (waitpid(pid, NULL, 0) == -1) {
			perror("waitpid");
			exit(EXIT_FAILURE);
		}
		printf("--------Child process exited--------\n");

		// Check mounts in the parent namespace after child exits.
		printf("Check old namespace mounts:\n");
		list_dir("/mnt");
		list_dir("/ext2");
	}

	return 0;
}
