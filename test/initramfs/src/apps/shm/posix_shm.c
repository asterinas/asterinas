// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/shm.h>
#include <sys/stat.h>
#include <sys/mman.h>

// ============================================================================
// Helper macros
// ============================================================================

#define SHM_OBJ "shm_test" // Name of shared memory object
#define SHM_SIZE 0x1000 // Size (in bytes) of shared memory object

#define THROW_ERROR(msg)            \
	do {                        \
		perror(msg);        \
		exit(EXIT_FAILURE); \
	} while (0)

/* Messages to communicate through shared memory */
#define MSG0 "1st Hello"
#define MSG1 "2nd Hello"
#define MSG2 "3rd Hello"
#define MSG3 "4th Hello"
#define MSG_SIZE (strlen(MSG0) + 1)

// ============================================================================
// Test functions for producer and consumer to communicate through shared memory
// ============================================================================

int producer_process()
{
	// Shared memory file descriptor
	int shm_fd;
	// Shared memory buffer
	void *shm_buf;

	// Create the shared memory object
	shm_fd = shm_open(SHM_OBJ, O_CREAT | O_RDWR, 0666);
	if (shm_fd < 0) {
		THROW_ERROR("shm_open failed");
	}

	// Configure the size of the shared memory object
	if (ftruncate(shm_fd, SHM_SIZE) < 0) {
		THROW_ERROR("ftruncate error");
	}

	// Memory map the shared memory object
	shm_buf = mmap(NULL, SHM_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED,
		       shm_fd, 0);
	if (shm_buf == MAP_FAILED) {
		THROW_ERROR("mmap(MAP_SHARED) failed");
	}

	// Fork a child and launch consumer process
	pid_t child_pid = fork();
	if (child_pid < 0) {
		THROW_ERROR("Spawn a child process failed");
	} else if (child_pid == 0) {
		execl("/test/shm/posix_shm", "posix_shm", "consumer", NULL);
		THROW_ERROR("exec failed");
	}

	// Communicate through shared memory
	strncpy(shm_buf, MSG0, MSG_SIZE);
	printf("[Producer] send %s\n", MSG0);
	while (1) {
		if (strncmp(shm_buf, MSG1, MSG_SIZE) != 0) {
			sleep(1);
			continue;
		}
		printf("[Producer] receive %s\n", MSG1);
		strncpy(shm_buf, MSG2, MSG_SIZE);
		printf("[Producer] send %s\n", MSG2);
		while (1) {
			if (strncmp(shm_buf, MSG3, MSG_SIZE) != 0) {
				sleep(1);
				continue;
			}
			printf("[Producer] receive %s\n", MSG3);
			break;
		}
		break;
	}

	// Unmap the shared memory
	if (munmap(shm_buf, SHM_SIZE) < 0) {
		THROW_ERROR("munmap failed");
	}

	// Unlink the shared memory object
	shm_unlink(SHM_OBJ);
	return 0;
}

int consumer_process()
{
	// Shared memory file descriptor
	int shm_fd;
	// Shared memory buffer
	void *shm_buf;

	// Create the shared memory object
	shm_fd = shm_open(SHM_OBJ, O_CREAT | O_RDWR, 0666);
	if (shm_fd < 0) {
		THROW_ERROR("shm_open failed");
	}

	// Configure the size of the shared memory object
	if (ftruncate(shm_fd, SHM_SIZE) < 0) {
		THROW_ERROR("ftruncate error");
	}

	// Memory map the shared memory object
	shm_buf = mmap(NULL, SHM_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED,
		       shm_fd, 0);
	if (shm_buf == MAP_FAILED) {
		THROW_ERROR("mmap(MAP_SHARED) failed");
	}

	while (1) {
		if (strncmp(shm_buf, MSG0, MSG_SIZE) != 0) {
			sleep(1);
			continue;
		}
		printf("[Consumer] receive %s\n", MSG0);
		strncpy(shm_buf, MSG1, MSG_SIZE);
		printf("[Consumer] send %s\n", MSG1);
		while (1) {
			if (strncmp(shm_buf, MSG2, MSG_SIZE) != 0) {
				sleep(1);
				continue;
			}
			printf("[Consumer] receive %s\n", MSG2);
			strncpy(shm_buf, MSG3, MSG_SIZE);
			printf("[Consumer] send %s\n", MSG3);
			break;
		}
		break;
	}

	// Unmap the shared memory
	if (munmap(shm_buf, SHM_SIZE) < 0) {
		THROW_ERROR("munmap failed");
	}

	// Unlink the shared memory object
	shm_unlink(SHM_OBJ);
	return 0;
}

// ============================================================================
// Test entry
// ============================================================================

int main(int argc, const char *argv[])
{
	if (argc == 1) {
		// Producer side
		return producer_process();
	} else {
		// Consumer side
		return consumer_process();
	}
}
