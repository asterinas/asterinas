// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>
#include <time.h>

#define DEVICE_PATH "/dev/nvme0"
#define BUFFER_SIZE 65536

int main()
{
	int fd;
	char write_buf[BUFFER_SIZE];
	char read_buf[BUFFER_SIZE];

	srand(time(NULL));

	for (size_t i = 0; i < BUFFER_SIZE; i++) {
		write_buf[i] = rand() % 256;
	}

	fd = open(DEVICE_PATH, O_RDWR);
	if (fd < 0) {
		perror("open");
		return EXIT_FAILURE;
	}

	ssize_t written = write(fd, write_buf, BUFFER_SIZE);
	if (written < 0) {
		perror("write");
		close(fd);
		return EXIT_FAILURE;
	}
	printf("Successfully wrote %zd bytes to %s\n", written, DEVICE_PATH);

	if (lseek(fd, 0, SEEK_SET) < 0) {
		perror("lseek");
		close(fd);
		return EXIT_FAILURE;
	}

	ssize_t read_bytes = read(fd, read_buf, BUFFER_SIZE);
	if (read_bytes < 0) {
		perror("read");
		close(fd);
		return EXIT_FAILURE;
	}
	printf("Successfully read %zd bytes from %s\n", read_bytes,
	       DEVICE_PATH);

	if (memcmp(write_buf, read_buf, BUFFER_SIZE) == 0) {
		printf("Successfully pass data verification! Read and Write match.\n");
	} else {
		printf("[ERROR]: Data verification failed! Read and Write do NOT match.\n");
	}

	close(fd);
	return EXIT_SUCCESS;
}
