// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/types.h>
#include <sys/ipc.h>
#include <sys/shm.h>
#include <sys/wait.h>
#include <time.h>

#define SHM_SIZE 4096
#define NUM_OF_CALLS 2
unsigned int SHM_KEY = 0x1234;

void test_shmget()
{
	struct timespec start, end;
	long seconds, nanoseconds, total_nanoseconds, avg_latency;
	int shmid;

	printf("\nTesting shmget()...\n");
	clock_gettime(CLOCK_MONOTONIC, &start);

	for (int i = 0; i < NUM_OF_CALLS; i++) {
		shmid = shmget(SHM_KEY, SHM_SIZE, IPC_CREAT | 0666);
		if (shmid == -1) {
			perror("shmget failed");
			exit(1);
		}
		// Delete the shared memory segment for the next test
		shmctl(shmid, IPC_RMID, NULL);
	}

	clock_gettime(CLOCK_MONOTONIC, &end);
	seconds = end.tv_sec - start.tv_sec;
	nanoseconds = end.tv_nsec - start.tv_nsec;
	total_nanoseconds = seconds * 1e9 + nanoseconds;
	avg_latency = total_nanoseconds / NUM_OF_CALLS;

	printf("Executed shmget() %d times.\n", NUM_OF_CALLS);
	printf("Average latency: %ld ns.\n", avg_latency);
}

void test_shmat_shmdt()
{
	struct timespec start, end;
	long seconds, nanoseconds, total_nanoseconds, avg_latency;
	int shmid;
	void *shmaddr;
	pid_t pid;
	char test_message[] = "Hello from shared memory!";
	char read_buffer[SHM_SIZE];

	printf("\nTesting shmat() and shmdt()...\n");

	// Create a shared memory segment
	shmid = shmget(SHM_KEY, SHM_SIZE, IPC_CREAT | 0666);
	if (shmid == -1) {
		perror("shmget failed");
		exit(1);
	}

	clock_gettime(CLOCK_MONOTONIC, &start);

	for (int i = 0; i < NUM_OF_CALLS; i++) {
		// Create child process
		pid = fork();
		if (pid < 0) {
			perror("fork failed");
			exit(1);
		}

		if (pid == 0) { // Child process
			// Test shmat
			shmaddr = shmat(shmid, NULL, 0);
			if (shmaddr == (void *)-1) {
				perror("shmat failed");
				exit(1);
			}

			// Write to shared memory
			strncpy((char *)shmaddr, test_message,
				strlen(test_message) + 1);

			// Test shmdt
			if (shmdt(shmaddr) == -1) {
				perror("shmdt failed");
				exit(1);
			}
			exit(0);
		} else { // Parent process
			// Test shmat
			shmaddr = shmat(shmid, NULL, 0);
			if (shmaddr == (void *)-1) {
				perror("shmat failed");
				exit(1);
			}

			// Wait for child to finish writing
			wait(NULL);

			// Read from shared memory
			strncpy(read_buffer, (char *)shmaddr,
				strlen(test_message) + 1);
			printf("Read from shared memory: %s\n", read_buffer);

			// Verify the content
			if (strcmp(read_buffer, test_message) != 0) {
				printf("Error: Shared memory content mismatch!\n");
				exit(1);
			}
		}

		// Test shmdt
		if (shmdt(shmaddr) == -1) {
			perror("shmdt failed");
			exit(1);
		}
	}

	clock_gettime(CLOCK_MONOTONIC, &end);
	seconds = end.tv_sec - start.tv_sec;
	nanoseconds = end.tv_nsec - start.tv_nsec;
	total_nanoseconds = seconds * 1e9 + nanoseconds;
	avg_latency = total_nanoseconds / NUM_OF_CALLS;

	printf("Executed shmat() and shmdt() %d times.\n", NUM_OF_CALLS);
	printf("Average latency: %ld ns.\n", avg_latency);

	// Clean up
	shmctl(shmid, IPC_RMID, NULL);
}

void test_shmctl()
{
	struct timespec start, end;
	long seconds, nanoseconds, total_nanoseconds, avg_latency;
	int shmid;
	struct shmid_ds buf;

	printf("\nTesting shmctl()...\n");

	// Create a shared memory segment
	shmid = shmget(SHM_KEY, SHM_SIZE, IPC_CREAT | 0666);
	if (shmid == -1) {
		perror("shmget failed");
		exit(1);
	}

	clock_gettime(CLOCK_MONOTONIC, &start);

	for (int i = 0; i < NUM_OF_CALLS; i++) {
		// Attach the shared memory segment
		void *shmaddr = shmat(shmid, NULL, 0);
		if (shmaddr == (void *)-1) {
			perror("shmat failed");
			exit(1);
		}

		// Test IPC_STAT
		if (shmctl(shmid, IPC_STAT, &buf) == -1) {
			perror("shmctl IPC_STAT failed");
			exit(1);
		}

		// Print buffer contents
		printf("IPC_STAT info:\n");
		printf("  shm_perm.uid: %d\n", buf.shm_perm.uid);
		printf("  shm_perm.gid: %d\n", buf.shm_perm.gid);
		printf("  shm_perm.mode: %o\n", buf.shm_perm.mode);
		printf("  shm_segsz: %ld\n", buf.shm_segsz);
		printf("  shm_nattch: %lu\n", buf.shm_nattch);

		// Test IPC_SET
		buf.shm_perm.mode = 0644; // Change mode to rw-r--r--
		if (shmctl(shmid, IPC_SET, &buf) == -1) {
			perror("shmctl IPC_SET failed");
			exit(1);
		}

		// Verify the change
		if (shmctl(shmid, IPC_STAT, &buf) == -1) {
			perror("shmctl IPC_STAT failed after IPC_SET");
			exit(1);
		}
		printf("After IPC_SET - new mode: %o\n", buf.shm_perm.mode);
		shmdt(shmaddr);
	}

	clock_gettime(CLOCK_MONOTONIC, &end);
	seconds = end.tv_sec - start.tv_sec;
	nanoseconds = end.tv_nsec - start.tv_nsec;
	total_nanoseconds = seconds * 1e9 + nanoseconds;
	avg_latency = total_nanoseconds / NUM_OF_CALLS;

	printf("Executed shmctl(IPC_STAT) %d times.\n", NUM_OF_CALLS);
	printf("Average latency: %ld ns.\n", avg_latency);

	// Clean up
	shmctl(shmid, IPC_RMID, NULL);
}

int main()
{
	printf("Starting shared memory system call tests...\n");

	printf("Testing with SHM_KEY = %d\n", SHM_KEY);
	SHM_KEY = 0x1234;
	test_shmget();
	test_shmat_shmdt();
	test_shmctl();

	printf("\nTesting with Anonymous SHM\n");
	SHM_KEY = 0;
	test_shmget();
	test_shmat_shmdt();
	test_shmctl();

	printf("\nAll tests completed.\n");
	return 0;
}