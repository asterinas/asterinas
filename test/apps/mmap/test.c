// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <unistd.h>
#include <pthread.h>
#include <string.h>
#include <sys/wait.h>
#include <fcntl.h>

#define REGION_SIZE (512 * 1024 * 1024) // 512 MB
#define THREAD_COUNT 4

typedef struct {
	char *base;
	size_t offset;
	size_t length;
} thread_arg_t;

void *page_fault_worker(void *arg)
{
	thread_arg_t *targ = (thread_arg_t *)arg;
	char *start = targ->base + targ->offset;
	char *end = start + targ->length;

	for (char *p = start; p < end; p += 4096) {
		volatile char c = *p;
		c++;
	}

	return NULL;
}

int main()
{
	const char *filename = "large_file";
	int fd = open(filename, O_RDWR | O_CREAT, 0666);
	if (fd == -1) {
		perror("open failed");
		return 1;
	}

	if (ftruncate(fd, REGION_SIZE) == -1) {
		perror("ftruncate failed");
		close(fd);
		return 1;
	}

	char *region = mmap(NULL, REGION_SIZE, PROT_READ, MAP_PRIVATE, fd, 0);
	if (region == MAP_FAILED) {
		perror("mmap failed");
		exit(1);
	}

	pthread_t threads[THREAD_COUNT];
	thread_arg_t args[THREAD_COUNT];
	size_t segment = REGION_SIZE / THREAD_COUNT;

	for (int i = 0; i < THREAD_COUNT; ++i) {
		args[i].base = region;
		args[i].offset = i * segment;
		args[i].length = segment;

		if (pthread_create(&threads[i], NULL, page_fault_worker,
				   &args[i]) != 0) {
			perror("pthread_create");
			exit(1);
		}
	}

	for (int i = 0; i < THREAD_COUNT; ++i) {
		pthread_join(threads[i], NULL);
	}

	pid_t pid = fork();

	if (pid < 0) {
		perror("fork failed");
		exit(1);
	}

	if (pid > 0) {
		printf("Parent PID: %d\n", getpid());
		while (1)
			pause();
	} else {
		printf("Child PID: %d\n", getpid());
		munmap(region, REGION_SIZE >> 1);
		while (1)
			pause();
	}

	return 0;
}