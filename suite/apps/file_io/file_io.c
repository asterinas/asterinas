// SPDX-License-Identifier: MPL-2.0

#include <sys/types.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <time.h>

#define KB 1024
#define MB (1024 * KB)
#define BUFFER_SIZE (4 * KB)
#define FILE_SIZE (256 * MB)
#define NUM_OF_CALLS 1000000

int fill_file(int fd)
{
	ssize_t bytes_write, offset = 0;
	char buffer[BUFFER_SIZE];

	memset(buffer, 0, BUFFER_SIZE);
	lseek(fd, 0, SEEK_SET);

	while (offset < FILE_SIZE) {
		bytes_write = write(fd, buffer, BUFFER_SIZE);
		if (bytes_write == -1) {
			fprintf(stderr, "Failed to write the file.\n");
			return -1;
		}
		offset += bytes_write;
	}

	return 0;
}

long calc_duration(struct timespec *start, struct timespec *end)
{
	return (end->tv_sec - start->tv_sec) * 1e9 +
	       (end->tv_nsec - start->tv_nsec);
}

int perform_sequential_io(int fd, ssize_t (*io_func)(int, void *, size_t),
			  const char *op_name)
{
	struct timespec start, end;
	char buffer[BUFFER_SIZE];
	ssize_t ret, offset = 0;
	long total_nanoseconds = 0, avg_latency;
	double throughput;

	memset(buffer, 0, BUFFER_SIZE);
	lseek(fd, 0, SEEK_SET);

	for (int i = 0; i < NUM_OF_CALLS; i++) {
		if (offset >= FILE_SIZE) {
			offset = lseek(fd, 0, SEEK_SET);
		}
		clock_gettime(CLOCK_MONOTONIC, &start);
		ret = io_func(fd, buffer, BUFFER_SIZE);
		clock_gettime(CLOCK_MONOTONIC, &end);
		if (ret == -1) {
			fprintf(stderr, "Failed to %s the file.\n", op_name);
			return -1;
		}
		offset += ret;
		total_nanoseconds += calc_duration(&start, &end);
	}

	avg_latency = total_nanoseconds / NUM_OF_CALLS;
	throughput = ((double)BUFFER_SIZE * NUM_OF_CALLS) /
		     ((double)total_nanoseconds / 1e9);
	printf("Executed the sequential %s (buffer size: %dKB, file size: %dMB) syscall %d times.\n",
	       op_name, BUFFER_SIZE / KB, FILE_SIZE / MB, NUM_OF_CALLS);
	printf("Syscall average latency: %ld nanoseconds, throughput: %.2f MB/s\n",
	       avg_latency, throughput / MB);

	return 0;
}

int sequential_read(int fd)
{
	return perform_sequential_io(fd, read, "read");
}

int sequential_write(int fd)
{
	return perform_sequential_io(fd, write, "write");
}

int main(int argc, char *argv[])
{
	if (argc < 2) {
		fprintf(stderr, "Usage: %s <file_name>\n", argv[0]);
		return -1;
	}

	int fd = open(argv[1], O_RDWR | O_CREAT, 00666);
	if (fd == -1) {
		fprintf(stderr, "Failed to open the file: %s.\n", argv[1]);
		return -1;
	}
	if (ftruncate(fd, FILE_SIZE) < 0) {
		fprintf(stderr,
			"Failed to truncate the file: %s to size: %dMB.\n",
			argv[1], FILE_SIZE / MB);
		return -1;
	}

	// Warm up by filling the file.
	if (fill_file(fd) < 0) {
		fprintf(stderr, "Failed to fill the file: %s.\n", argv[1]);
		return -1;
	}

	if (sequential_read(fd) < 0) {
		fprintf(stderr,
			"Failed to do sequential read on the file: %s.\n",
			argv[1]);
		return -1;
	}
	if (sequential_write(fd) < 0) {
		fprintf(stderr,
			"Failed to do sequential write on the file: %s.\n",
			argv[1]);
		return -1;
	}

	// TODO: Add more test cases such as random read and random write.

	close(fd);
	if (unlink(argv[1]) < 0) {
		fprintf(stderr, "Failed to delete the file: %s.\n", argv[1]);
		return -1;
	}

	return 0;
}