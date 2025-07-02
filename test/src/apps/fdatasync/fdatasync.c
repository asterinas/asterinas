// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <unistd.h>
#include <fcntl.h>
#include <string.h>
#include <stdlib.h>

void test_fdatasync_on_fs(const char *directory)
{
	char filepath[256];
	snprintf(filepath, sizeof(filepath), "%s/test_fdatasync.txt",
		 directory);

	int fd =
		open(filepath, O_WRONLY | O_CREAT | O_TRUNC, S_IRUSR | S_IWUSR);
	if (fd == -1) {
		perror("Error opening file");
		exit(EXIT_FAILURE);
	}

	char *data = "Hello, fdatasync test!\n";
	if (write(fd, data, strlen(data)) != strlen(data)) {
		perror("Error writing data");
		close(fd);
		exit(EXIT_FAILURE);
	}

	if (fdatasync(fd) == -1) {
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

	test_fdatasync_on_fs(argv[1]);

	return EXIT_SUCCESS;
}
