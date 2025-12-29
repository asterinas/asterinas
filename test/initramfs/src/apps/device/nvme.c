// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#define DEVICE_PATH "/dev/nvme0"
#define MOUNT_POINT "/nvme"
#define TEST_FILE "/nvme/nvme_test"
#define BUFFER_SIZE 65536
#define ALIGNMENT 4096

#ifndef O_DIRECT
#define O_DIRECT 040000
#endif

static int ensure_mount_point(void)
{
	if (mkdir(MOUNT_POINT, 0755) == 0) {
		return 0;
	}
	if (errno == EEXIST) {
		return 0;
	}
	perror("mkdir");
	return -1;
}

int main(void)
{
	int fd;
	char *write_buf = NULL;
	char *read_buf = NULL;

	if (ensure_mount_point() != 0) {
		return EXIT_FAILURE;
	}

	if (mount(DEVICE_PATH, MOUNT_POINT, "ext2", 0, "") < 0) {
		perror("mount");
		return EXIT_FAILURE;
	}

	if (posix_memalign((void **)&write_buf, ALIGNMENT, BUFFER_SIZE) != 0 ||
	    posix_memalign((void **)&read_buf, ALIGNMENT, BUFFER_SIZE) != 0) {
		perror("posix_memalign");
		umount(MOUNT_POINT);
		return EXIT_FAILURE;
	}

	srand(time(NULL));
	for (size_t i = 0; i < BUFFER_SIZE; i++) {
		write_buf[i] = rand() % 256;
	}

	fd = open(TEST_FILE, O_CREAT | O_TRUNC | O_RDWR | O_DIRECT, 0644);
	if (fd < 0) {
		perror("open");
		free(write_buf);
		free(read_buf);
		umount(MOUNT_POINT);
		return EXIT_FAILURE;
	}

	ssize_t written = write(fd, write_buf, BUFFER_SIZE);
	if (written < 0) {
		perror("write");
		close(fd);
		free(write_buf);
		free(read_buf);
		umount(MOUNT_POINT);
		return EXIT_FAILURE;
	}
	printf("Successfully wrote %zd bytes to %s\n", written, TEST_FILE);

	if (lseek(fd, 0, SEEK_SET) < 0) {
		perror("lseek");
		close(fd);
		free(write_buf);
		free(read_buf);
		umount(MOUNT_POINT);
		return EXIT_FAILURE;
	}

	ssize_t read_bytes = read(fd, read_buf, BUFFER_SIZE);
	if (read_bytes < 0) {
		perror("read");
		close(fd);
		free(write_buf);
		free(read_buf);
		umount(MOUNT_POINT);
		return EXIT_FAILURE;
	}
	printf("Successfully read %zd bytes from %s\n", read_bytes, TEST_FILE);

	if (memcmp(write_buf, read_buf, BUFFER_SIZE) == 0) {
		printf("Successfully pass data verification! Read and Write match.\n");
	} else {
		printf("[ERROR]: Data verification failed! Read and Write do NOT match.\n");
	}

	close(fd);
	free(write_buf);
	free(read_buf);
	if (umount(MOUNT_POINT) < 0) {
		perror("umount");
		return EXIT_FAILURE;
	}

	return EXIT_SUCCESS;
}
