// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <unistd.h>
#include <fcntl.h>
#include <string.h>
#include <stdlib.h>

static void test_sync_on_fs(const char *directory, int data_only)
{
	char filepath[256];
	snprintf(filepath, sizeof(filepath), "%s/test_%s.txt", directory,
		 data_only ? "fdatasync" : "fsync");

	int fd =
		open(filepath, O_WRONLY | O_CREAT | O_TRUNC, S_IRUSR | S_IWUSR);
	if (fd == -1) {
		perror("Error opening file");
		exit(EXIT_FAILURE);
	}

	char *data = "Hello, sync test!\n";
	if (write(fd, data, strlen(data)) != strlen(data)) {
		perror("Error writing data");
		close(fd);
		exit(EXIT_FAILURE);
	}

	if ((data_only ? fdatasync(fd) : fsync(fd)) == -1) {
		perror("Error syncing data");
		close(fd);
		exit(EXIT_FAILURE);
	}

	printf("Data written and synced on %s\n", directory);
	close(fd);
}

int main(int argc, char **argv)
{
	if (argc != 2) {
		printf("Usage: %s <directory>\n", argv[0]);
		return EXIT_FAILURE;
	}

	test_sync_on_fs(argv[1], 1);
	test_sync_on_fs(argv[1], 0);

	return EXIT_SUCCESS;
}
