// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <sched.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ipc.h>
#include <sys/sem.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define STACK_SIZE (1024 * 1024)

// A unique key for semaphore creation.
#define SEM_KEY_BASE 0x1234

union semun {
	int val;
	struct semid_ds *buf;
	unsigned short *array;
};

// --- Helper functions ---

// Creates a semaphore set with 1 semaphore and returns its ID.
// Returns -1 on failure.
static int create_sem(int key)
{
	return semget(key, 1, IPC_CREAT | IPC_EXCL | 0666);
}

// Tries to get an existing semaphore set by key.
// Returns the semaphore ID, or -1 if not found.
static int get_sem(int key)
{
	return semget(key, 1, 0666);
}

// Removes a semaphore set by ID.
static int remove_sem(int semid)
{
	union semun arg;
	return semctl(semid, 0, IPC_RMID, arg);
}

// Sets the value of a semaphore.
static int set_sem_val(int semid, int val)
{
	union semun arg;
	arg.val = val;
	return semctl(semid, 0, SETVAL, arg);
}

// Gets the value of a semaphore.
static int get_sem_val(int semid)
{
	return semctl(semid, 0, GETVAL);
}

// --- Test: Semaphore sharing within the same IPC namespace ---
//
// A parent creates a semaphore, then forks a child (without CLONE_NEWIPC).
// The child should be able to see and modify the same semaphore.

FN_TEST(sem_shared_in_same_ipc_ns)
{
	int key = SEM_KEY_BASE + 1;
	int semid = TEST_SUCC(create_sem(key));
	TEST_SUCC(set_sem_val(semid, 10));

	pid_t pid = TEST_SUCC(fork());

	if (pid == 0) {
		// Child: should see the same semaphore
		int child_semid = get_sem(key);
		if (child_semid < 0) {
			fprintf(stderr,
				"child: failed to find semaphore in same IPC ns\n");
			_exit(1);
		}
		int val = semctl(child_semid, 0, GETVAL);
		if (val != 10) {
			fprintf(stderr, "child: expected sem val 10, got %d\n",
				val);
			_exit(1);
		}
		// Modify the value
		union semun arg;
		arg.val = 20;
		if (semctl(child_semid, 0, SETVAL, arg) < 0) {
			fprintf(stderr, "child: failed to set sem val\n");
			_exit(1);
		}
		_exit(0);
	}

	int status;
	TEST_SUCC(waitpid(pid, &status, 0));
	TEST_RES(WIFEXITED(status) && WEXITSTATUS(status), _ret == 0);

	// Parent: verify the child's modification is visible
	TEST_RES(get_sem_val(semid), _ret == 20);

	TEST_SUCC(remove_sem(semid));
}
END_TEST()

// --- Test: Semaphore isolation via unshare(CLONE_NEWIPC) ---
//
// A parent creates a semaphore, then the child calls unshare(CLONE_NEWIPC).
// After unshare, the child should NOT see the parent's semaphore,
// and a semaphore created by the child should NOT be visible to the parent.

FN_TEST(sem_isolation_via_unshare)
{
	int parent_key = SEM_KEY_BASE + 2;
	int child_key = SEM_KEY_BASE + 3;

	int parent_semid = TEST_SUCC(create_sem(parent_key));
	TEST_SUCC(set_sem_val(parent_semid, 42));

	pid_t pid = TEST_SUCC(fork());

	if (pid == 0) {
		// Child: unshare IPC namespace
		if (unshare(CLONE_NEWIPC) < 0) {
			fprintf(stderr,
				"child: unshare(CLONE_NEWIPC) failed: %s\n",
				strerror(errno));
			_exit(1);
		}

		// The parent's semaphore should NOT be visible
		int ret = get_sem(parent_key);
		if (ret >= 0) {
			fprintf(stderr,
				"child: parent's semaphore should not be visible after unshare\n");
			_exit(1);
		}

		// Create a new semaphore in the child's IPC namespace
		int child_semid = create_sem(child_key);
		if (child_semid < 0) {
			fprintf(stderr,
				"child: failed to create semaphore in new IPC ns: %s\n",
				strerror(errno));
			_exit(1);
		}

		union semun arg;
		arg.val = 99;
		if (semctl(child_semid, 0, SETVAL, arg) < 0) {
			_exit(1);
		}

		// Clean up child's semaphore
		semctl(child_semid, 0, IPC_RMID, arg);
		_exit(0);
	}

	int status;
	TEST_SUCC(waitpid(pid, &status, 0));
	TEST_RES(WIFEXITED(status) && WEXITSTATUS(status), _ret == 0);

	// Parent: the child's semaphore should NOT be visible
	TEST_ERRNO(get_sem(child_key), ENOENT);

	// Parent: own semaphore should still be accessible
	TEST_RES(get_sem_val(parent_semid), _ret == 42);

	TEST_SUCC(remove_sem(parent_semid));
}
END_TEST()

// --- Test: Semaphore isolation via clone(CLONE_NEWIPC) ---
//
// Clone a child into a new IPC namespace. The child should start with
// an empty set of IPC resources and should not see the parent's semaphore.

static int clone_newipc_child_fn(void *arg)
{
	int parent_key = *(int *)arg;

	// The parent's semaphore should NOT be visible
	int ret = get_sem(parent_key);
	if (ret >= 0) {
		fprintf(stderr,
			"clone child: parent's semaphore should not be visible\n");
		return 1;
	}

	// Create a new semaphore in the child's own namespace
	int child_key = SEM_KEY_BASE + 5;
	int child_semid = create_sem(child_key);
	if (child_semid < 0) {
		fprintf(stderr, "clone child: failed to create semaphore: %s\n",
			strerror(errno));
		return 1;
	}

	union semun sarg;
	sarg.val = 77;
	if (semctl(child_semid, 0, SETVAL, sarg) < 0) {
		return 1;
	}

	// Clean up
	semctl(child_semid, 0, IPC_RMID, sarg);
	return 0;
}

FN_TEST(sem_isolation_via_clone)
{
	int parent_key = SEM_KEY_BASE + 4;
	int parent_semid = TEST_SUCC(create_sem(parent_key));
	TEST_SUCC(set_sem_val(parent_semid, 55));

	char *stack = malloc(STACK_SIZE);
	char *stack_top = stack + STACK_SIZE;

	pid_t pid = TEST_SUCC(clone(clone_newipc_child_fn, stack_top,
				    CLONE_NEWIPC | SIGCHLD, &parent_key));

	int status;
	TEST_SUCC(waitpid(pid, &status, 0));
	TEST_RES(WIFEXITED(status) && WEXITSTATUS(status), _ret == 0);

	// Parent: own semaphore should still be intact
	TEST_RES(get_sem_val(parent_semid), _ret == 55);

	// Parent: the child's semaphore (key SEM_KEY_BASE + 5) should NOT be visible
	int child_key = SEM_KEY_BASE + 5;
	TEST_ERRNO(get_sem(child_key), ENOENT);

	free(stack);
	TEST_SUCC(remove_sem(parent_semid));
}
END_TEST()

// --- Test: Independent semaphore ID allocation across IPC namespaces ---
//
// Two children in separate IPC namespaces should be able to create semaphores
// with the same key without conflict.

static int clone_independent_child_fn(void *arg)
{
	int key = *(int *)arg;

	int semid = create_sem(key);
	if (semid < 0) {
		fprintf(stderr,
			"independent child: failed to create semaphore with key %d: %s\n",
			key, strerror(errno));
		return 1;
	}

	union semun sarg;
	sarg.val = 123;
	if (semctl(semid, 0, SETVAL, sarg) < 0) {
		return 1;
	}

	// Clean up
	semctl(semid, 0, IPC_RMID, sarg);
	return 0;
}

FN_TEST(sem_independent_across_namespaces)
{
	int shared_key = SEM_KEY_BASE + 6;

	char *stack1 = malloc(STACK_SIZE);
	char *stack1_top = stack1 + STACK_SIZE;
	char *stack2 = malloc(STACK_SIZE);
	char *stack2_top = stack2 + STACK_SIZE;

	// Clone two children, each in a new IPC namespace
	pid_t pid1 = TEST_SUCC(clone(clone_independent_child_fn, stack1_top,
				     CLONE_NEWIPC | SIGCHLD, &shared_key));

	pid_t pid2 = TEST_SUCC(clone(clone_independent_child_fn, stack2_top,
				     CLONE_NEWIPC | SIGCHLD, &shared_key));

	int status;
	TEST_SUCC(waitpid(pid1, &status, 0));
	TEST_RES(WIFEXITED(status) && WEXITSTATUS(status), _ret == 0);

	TEST_SUCC(waitpid(pid2, &status, 0));
	TEST_RES(WIFEXITED(status) && WEXITSTATUS(status), _ret == 0);

	free(stack1);
	free(stack2);
}
END_TEST()

// --- Test: Semaphore operations (semop) work correctly in a new IPC namespace ---
//
// Clone a child into a new IPC namespace. The child creates a semaphore, performs
// semop (V then P operations), and verifies values.

static int clone_semop_child_fn(void *arg)
{
	(void)arg;

	int key = SEM_KEY_BASE + 7;
	int semid = create_sem(key);
	if (semid < 0) {
		fprintf(stderr, "semop child: failed to create semaphore: %s\n",
			strerror(errno));
		return 1;
	}

	// Initialize semaphore value to 0
	union semun sarg;
	sarg.val = 0;
	if (semctl(semid, 0, SETVAL, sarg) < 0) {
		return 1;
	}

	// Perform V operation (increment by 3)
	struct sembuf sop;
	sop.sem_num = 0;
	sop.sem_op = 3;
	sop.sem_flg = 0;
	if (semop(semid, &sop, 1) < 0) {
		fprintf(stderr, "semop child: V operation failed: %s\n",
			strerror(errno));
		return 1;
	}

	// Verify value is 3
	int val = semctl(semid, 0, GETVAL);
	if (val != 3) {
		fprintf(stderr, "semop child: expected 3, got %d\n", val);
		return 1;
	}

	// Perform P operation (decrement by 1)
	sop.sem_op = -1;
	if (semop(semid, &sop, 1) < 0) {
		fprintf(stderr, "semop child: P operation failed: %s\n",
			strerror(errno));
		return 1;
	}

	// Verify value is 2
	val = semctl(semid, 0, GETVAL);
	if (val != 2) {
		fprintf(stderr, "semop child: expected 2, got %d\n", val);
		return 1;
	}

	// Clean up
	semctl(semid, 0, IPC_RMID, sarg);
	return 0;
}

FN_TEST(sem_operations_in_new_ipc_ns)
{
	char *stack = malloc(STACK_SIZE);
	char *stack_top = stack + STACK_SIZE;

	pid_t pid = TEST_SUCC(clone(clone_semop_child_fn, stack_top,
				    CLONE_NEWIPC | SIGCHLD, NULL));

	int status;
	TEST_SUCC(waitpid(pid, &status, 0));
	TEST_RES(WIFEXITED(status) && WEXITSTATUS(status), _ret == 0);

	free(stack);
}
END_TEST()

// --- Test: IPC resources are cleaned up when namespace is destroyed ---
//
// A child creates a semaphore in a new IPC namespace and exits without
// explicitly removing it. After the child exits, the namespace should be
// destroyed and its resources freed. Verify the parent's namespace is unaffected.

FN_TEST(sem_cleanup_on_ns_destroy)
{
	int parent_key = SEM_KEY_BASE + 8;
	int parent_semid = TEST_SUCC(create_sem(parent_key));
	TEST_SUCC(set_sem_val(parent_semid, 100));

	pid_t pid = TEST_SUCC(fork());

	if (pid == 0) {
		if (unshare(CLONE_NEWIPC) < 0) {
			_exit(1);
		}

		// Create a semaphore in the new namespace and intentionally
		// do NOT remove it — it should be cleaned up when the
		// namespace is destroyed.
		int child_key = SEM_KEY_BASE + 9;
		int child_semid = create_sem(child_key);
		if (child_semid < 0) {
			_exit(1);
		}

		union semun arg;
		arg.val = 50;
		semctl(child_semid, 0, SETVAL, arg);

		// Exit without cleaning up
		_exit(0);
	}

	int status;
	TEST_SUCC(waitpid(pid, &status, 0));
	TEST_RES(WIFEXITED(status) && WEXITSTATUS(status), _ret == 0);

	// Parent's semaphore should still be intact
	TEST_RES(get_sem_val(parent_semid), _ret == 100);

	TEST_SUCC(remove_sem(parent_semid));
}
END_TEST()
