// SPDX-License-Identifier: MPL-2.0
// A regression test for the futex lost wakeup bug fixed in https://github.com/asterinas/asterinas/pull/1642

#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <signal.h>
#include <stdatomic.h>

pthread_mutex_t mutex;
atomic_int sync_flag = ATOMIC_VAR_INIT(0); // Atomic flag for synchronization

// Signal handler for SIGUSR1
void signal_handler(int signum)
{
	atomic_store(&sync_flag, 2);
}

// Thread function that tries to lock the mutex and waits if it is locked
void *thread_function(void *arg)
{
	printf("Thread: Trying to lock mutex...\n");

	// Set the atomic flag to signal the main thread
	atomic_store(&sync_flag, 1);

	// Try to lock the mutex
	pthread_mutex_lock(&mutex);
	printf("Thread: Got the mutex!\n");

	printf("Thread: Exiting.\n");
	pthread_mutex_unlock(&mutex);

	// Set the atomic flag to signal the main thread
	atomic_store(&sync_flag, 3);
	return NULL;
}

int main()
{
	pthread_t thread;

	// Initialize mutex
	if (pthread_mutex_init(&mutex, NULL) != 0) {
		perror("Mutex initialization failed");
		return -1;
	}

	// Set up signal handler for SIGUSR1
	struct sigaction sa;
	sa.sa_handler = signal_handler;
	sa.sa_flags = 0;
	sigemptyset(&sa.sa_mask);
	if (sigaction(SIGUSR1, &sa, NULL) == -1) {
		perror("sigaction failed");
		return -1;
	}

	// Main thread locks the mutex
	pthread_mutex_lock(&mutex);
	printf("Main thread: Mutex locked.\n");

	// Create the second thread
	if (pthread_create(&thread, NULL, thread_function, NULL) != 0) {
		perror("Thread creation failed");
		return -1;
	}

	// Detach the thread to allow it to run independently
	if (pthread_detach(thread) != 0) {
		perror("Thread detachment failed");
		return -1;
	}

	// Wait for the second thread to prepare
	while (atomic_load(&sync_flag) != 1) {
	}
	sleep(1);

	// Send signal to the second thread
	pthread_kill(thread, SIGUSR1);
	printf("Main thread: Signal sent to the thread.\n");

	// Wait for the second thread to process signal
	while (atomic_load(&sync_flag) != 2) {
	}
	sleep(1);

	// Unlock the mutex
	pthread_mutex_unlock(&mutex);
	printf("Main thread: Mutex unlocked.\n");

	// Wait for the second thread to exit
	int count = 3;
	while (atomic_load(&sync_flag) != 3 && count--) {
		sleep(1);
	}
	if (atomic_load(&sync_flag) != 3) {
		printf("ERROR: Thread does not exit after timeout.\n");
		exit(EXIT_FAILURE);
	}

	// Destroy mutex
	pthread_mutex_destroy(&mutex);

	printf("All tests passed.\n");
	return 0;
}
