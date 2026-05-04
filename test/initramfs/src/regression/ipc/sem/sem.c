// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <sys/ipc.h>
#include <sys/sem.h>

#include "../../common/test.h"

#define SEMMNI 320000
#define SEMMSL 320000

#define CUSTOM_KEY 0xdeadbeef

union semun {
	int val;
	struct semid_ds *buf;
	unsigned short *array;
};

static int create_sem_set(int nsems)
{
	return semget(IPC_PRIVATE, nsems, IPC_CREAT | 0600);
}

static int remove_sem_set(int semid)
{
	union semun arg = { 0 };

	return semctl(semid, 0, IPC_RMID, arg);
}

FN_TEST(semget_accept_arbitrary_keys)
{
	int semid = TEST_SUCC(semget(CUSTOM_KEY, 1, IPC_CREAT | 0600));
	int semid2;

	/*
	 * FIXME: Asterinas rejects keys that hash to the same ID.
	 */
#ifdef __asterinas__
	TEST_ERRNO(semget(CUSTOM_KEY + SEMMNI, 1, IPC_CREAT | 0600), ENOSPC);
	TEST_ERRNO(semget(CUSTOM_KEY + SEMMNI, 0, 0), ENOENT);
	semid2 = TEST_RES(semget(CUSTOM_KEY + 1, 1, IPC_CREAT | 0600),
			  _ret != semid);
#else
	TEST_ERRNO(semget(CUSTOM_KEY + SEMMNI, 0, 0), ENOENT);
	semid2 = TEST_RES(semget(CUSTOM_KEY + SEMMNI, 1, IPC_CREAT | 0600),
			  _ret != semid);
#endif

	TEST_RES(semget(CUSTOM_KEY, 0, IPC_CREAT | 0600), _ret == semid);
	TEST_RES(semget(CUSTOM_KEY, 1, IPC_CREAT | 0600), _ret == semid);
	TEST_ERRNO(semget(CUSTOM_KEY, 0, IPC_CREAT | IPC_EXCL | 0600), EEXIST);
	TEST_ERRNO(semget(CUSTOM_KEY, 1, IPC_CREAT | IPC_EXCL | 0600), EEXIST);

	TEST_ERRNO(semget(CUSTOM_KEY, 2, IPC_CREAT | 0600), EINVAL);
	TEST_ERRNO(semget(CUSTOM_KEY, -1, IPC_CREAT | 0600), EINVAL);
	TEST_ERRNO(semget(CUSTOM_KEY, 2, 0), EINVAL);
	TEST_ERRNO(semget(CUSTOM_KEY, -1, 0), EINVAL);

	TEST_SUCC(remove_sem_set(semid));
	TEST_SUCC(remove_sem_set(semid2));
}
END_TEST()

FN_TEST(semctl_ipc_commands_ignore_semnum)
{
	union semun arg = { 0 };
	struct semid_ds semid_ds;
	int semid = TEST_SUCC(create_sem_set(1));

	arg.buf = &semid_ds;
	TEST_SUCC(semctl(semid, -1, IPC_STAT, arg));
	TEST_SUCC(semctl(semid, -1, IPC_RMID, arg));
}
END_TEST()

FN_TEST(semctl_waiter_counts_reject_bad_semnum)
{
	union semun arg = { 0 };
	int semid = TEST_SUCC(create_sem_set(1));

	TEST_ERRNO(semctl(semid, -1, GETNCNT, arg), EINVAL);
	TEST_ERRNO(semctl(semid, 1, GETNCNT, arg), EINVAL);
	TEST_ERRNO(semctl(semid, -1, GETZCNT, arg), EINVAL);
	TEST_ERRNO(semctl(semid, 1, GETZCNT, arg), EINVAL);

	TEST_SUCC(remove_sem_set(semid));
}
END_TEST()
