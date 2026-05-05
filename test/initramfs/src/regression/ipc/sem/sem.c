// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <sys/ipc.h>
#include <sys/sem.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#include "../../common/test.h"

#define SEMMNI 320000
#define SEMMSL 320000
#define SEMVMX 32767

#define CUSTOM_KEY 0xdeadbeef

#define SETTLE_MS 100
#define SHORT_TIMEOUT_MS 1000
#define LONG_TIMEOUT_MS 4000

union semun {
	int val;
	struct semid_ds *buf;
	unsigned short *array;
};

struct timed_semop_args {
	int semid;
	struct sembuf ops[2];
	size_t nops;
	long timeout_ms;
	int error;
};

static struct timespec timespec_from_ms(long milliseconds)
{
	return (struct timespec){
		.tv_sec = milliseconds / 1000,
		.tv_nsec = (milliseconds % 1000) * 1000000L,
	};
}

static void sleep_ms(long milliseconds)
{
	struct timespec request = timespec_from_ms(milliseconds);

	CHECK(nanosleep(&request, NULL));
}

static int create_sem_set(int nsems)
{
	return semget(IPC_PRIVATE, nsems, IPC_CREAT | 0600);
}

static int remove_sem_set(int semid)
{
	union semun arg = { 0 };

	return semctl(semid, 0, IPC_RMID, arg);
}

static int get_sem_val(int semid, int semnum)
{
	union semun arg = { 0 };

	return semctl(semid, semnum, GETVAL, arg);
}

static int set_sem_val(int semid, int semnum, int val)
{
	union semun arg = {
		.val = val,
	};

	return semctl(semid, semnum, SETVAL, arg);
}

static int get_sem_ncnt(int semid, int semnum)
{
	union semun arg = { 0 };

	return semctl(semid, semnum, GETNCNT, arg);
}

static int get_sem_zcnt(int semid, int semnum)
{
	union semun arg = { 0 };

	return semctl(semid, semnum, GETZCNT, arg);
}

static void *timed_semop_thread(void *data)
{
	struct timed_semop_args *args = data;
	struct timespec timeout = timespec_from_ms(args->timeout_ms);

	errno = 0;
	if (syscall(SYS_semtimedop, args->semid, args->ops, args->nops,
		    &timeout) == 0) {
		args->error = 0;
	} else {
		args->error = errno;
	}

	return NULL;
}

static int start_timed_semop_thread(pthread_t *thread,
				    struct timed_semop_args *args)
{
	int error = pthread_create(thread, NULL, &timed_semop_thread, args);

	if (error != 0) {
		errno = error;
		return -1;
	}

	return 0;
}

static int join_timed_semop_thread(pthread_t thread,
				   const struct timed_semop_args *args)
{
	int error = pthread_join(thread, NULL);

	if (error != 0) {
		errno = error;
		return -1;
	}

	return args->error;
}

static void signal_handler(int signum)
{
	(void)signum;
}

FN_SETUP(install_signal_handler)
{
	struct sigaction action = {
		.sa_handler = signal_handler,
	};

	CHECK(sigemptyset(&action.sa_mask));
	CHECK(sigaction(SIGUSR1, &action, NULL));
}
END_SETUP()

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

FN_TEST(semctl_set_zeros_wake_pending_alterations)
{
	struct timed_semop_args wait = {
		.nops = 2,
		.timeout_ms = LONG_TIMEOUT_MS,
		.ops = { { .sem_num = 0, .sem_op = 0, .sem_flg = 0 },
			 { .sem_num = 1, .sem_op = -1, .sem_flg = 0 } },
	};
	pthread_t thread;
	int semid = TEST_SUCC(create_sem_set(2));

	TEST_SUCC(set_sem_val(semid, 0, 1));
	TEST_SUCC(set_sem_val(semid, 1, 1));

	wait.semid = semid;
	TEST_SUCC(start_timed_semop_thread(&thread, &wait));
	sleep_ms(SETTLE_MS);
	TEST_RES(get_sem_zcnt(semid, 0), _ret == 1);

	TEST_SUCC(set_sem_val(semid, 0, 0));
	TEST_RES(join_timed_semop_thread(thread, &wait), _ret == 0);
	TEST_RES(get_sem_val(semid, 0), _ret == 0);
	TEST_RES(get_sem_val(semid, 1), _ret == 0);

	TEST_SUCC(remove_sem_set(semid));
}
END_TEST()

FN_TEST(semop_timeout_keeps_same_process_waiters)
{
	struct timed_semop_args short_wait = {
		.nops = 1,
		.timeout_ms = SHORT_TIMEOUT_MS,
		.ops = { { .sem_num = 0, .sem_op = -1, .sem_flg = 0 } },
	};
	struct timed_semop_args long_wait = {
		.nops = 1,
		.timeout_ms = LONG_TIMEOUT_MS,
		.ops = { { .sem_num = 0, .sem_op = -1, .sem_flg = 0 } },
	};
	struct sembuf post = { .sem_num = 0, .sem_op = 1, .sem_flg = 0 };
	pthread_t short_thread;
	pthread_t long_thread;
	int semid = TEST_SUCC(create_sem_set(1));

	short_wait.semid = semid;
	TEST_SUCC(start_timed_semop_thread(&short_thread, &short_wait));
	sleep_ms(SETTLE_MS);
	TEST_RES(get_sem_ncnt(semid, 0), _ret == 1);

	long_wait.semid = semid;
	TEST_SUCC(start_timed_semop_thread(&long_thread, &long_wait));
	sleep_ms(SETTLE_MS);
	TEST_RES(get_sem_ncnt(semid, 0), _ret == 2);

	TEST_RES(join_timed_semop_thread(short_thread, &short_wait),
		 _ret == EAGAIN);
	TEST_RES(get_sem_ncnt(semid, 0), _ret == 1);

	TEST_SUCC(semop(semid, &post, 1));
	TEST_RES(join_timed_semop_thread(long_thread, &long_wait), _ret == 0);
	TEST_RES(get_sem_val(semid, 0), _ret == 0);

	TEST_SUCC(remove_sem_set(semid));
}
END_TEST()

FN_TEST(semop_is_interrupted_by_signal)
{
	struct sembuf wait = { .sem_num = 0, .sem_op = -1, .sem_flg = 0 };
	int semid = TEST_SUCC(create_sem_set(1));
	pid_t child = TEST_SUCC(fork());
	int status;

	if (child == 0) {
		CHECK_WITH(semop(semid, &wait, 1),
			   _ret == -1 && errno == EINTR);
		_exit(0);
	}

	sleep_ms(SETTLE_MS);
	TEST_RES(get_sem_ncnt(semid, 0), _ret == 1);
	TEST_SUCC(kill(child, SIGUSR1));

	TEST_RES(waitpid(child, &status, 0),
		 WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_RES(get_sem_ncnt(semid, 0), _ret == 0);

	TEST_SUCC(remove_sem_set(semid));
}
END_TEST()

FN_TEST(semop_duplicate_semnum)
{
	struct sembuf ops[2] = {
		{ .sem_num = 0, .sem_op = 1, .sem_flg = 0 },
		{ .sem_num = 0, .sem_op = -1, .sem_flg = IPC_NOWAIT },
	};
	int semid = TEST_SUCC(create_sem_set(1));

	/*
	 * FIXME: Linux supports multiple operations on the same semaphore in a
	 * single `semop`, but Asterinas rejects this unsupported case before
	 * applying any operation.
	 */
#ifdef __asterinas__
	TEST_ERRNO(semop(semid, ops, 2), EOPNOTSUPP);
#else
	TEST_SUCC(semop(semid, ops, 2));
#endif
	TEST_RES(get_sem_val(semid, 0), _ret == 0);

	TEST_SUCC(remove_sem_set(semid));
}
END_TEST()

FN_TEST(semop_sem_undo)
{
	struct sembuf op = {
		.sem_num = 0,
		.sem_op = 1,
		.sem_flg = SEM_UNDO,
	};
	int semid = TEST_SUCC(create_sem_set(1));

	/*
	 * FIXME: Linux applies `SEM_UNDO`, but Asterinas rejects the flag as it
	 * is not supported.
	 */
#ifdef __asterinas__
	TEST_ERRNO(semop(semid, &op, 1), EINVAL);
	TEST_RES(get_sem_val(semid, 0), _ret == 0);
#else
	TEST_SUCC(semop(semid, &op, 1));
	TEST_RES(get_sem_val(semid, 0), _ret == 1);
#endif

	TEST_SUCC(remove_sem_set(semid));
}
END_TEST()

FN_TEST(semop_retries_pending_alterations)
{
	struct timed_semop_args first_wait = {
		.nops = 1,
		.timeout_ms = LONG_TIMEOUT_MS,
		.ops = { { .sem_num = 0, .sem_op = -1, .sem_flg = 0 } },
	};
	struct timed_semop_args second_wait = {
		.nops = 2,
		.timeout_ms = LONG_TIMEOUT_MS,
		.ops = { { .sem_num = 0, .sem_op = 1, .sem_flg = 0 },
			 { .sem_num = 1, .sem_op = -1, .sem_flg = 0 } },
	};
	struct sembuf release_second = {
		.sem_num = 1,
		.sem_op = 1,
		.sem_flg = 0,
	};
	pthread_t first_thread;
	pthread_t second_thread;
	int semid = TEST_SUCC(create_sem_set(2));

	first_wait.semid = semid;
	TEST_SUCC(start_timed_semop_thread(&first_thread, &first_wait));
	sleep_ms(SETTLE_MS);
	TEST_RES(get_sem_ncnt(semid, 0), _ret == 1);

	second_wait.semid = semid;
	TEST_SUCC(start_timed_semop_thread(&second_thread, &second_wait));
	sleep_ms(SETTLE_MS);
	TEST_RES(get_sem_ncnt(semid, 1), _ret == 1);

	TEST_SUCC(semop(semid, &release_second, 1));
	TEST_RES(join_timed_semop_thread(second_thread, &second_wait),
		 _ret == 0);
	TEST_RES(join_timed_semop_thread(first_thread, &first_wait), _ret == 0);
	TEST_RES(get_sem_val(semid, 0), _ret == 0);
	TEST_RES(get_sem_val(semid, 1), _ret == 0);

	TEST_SUCC(remove_sem_set(semid));
}
END_TEST()

FN_TEST(semop_updates_pid_with_or_without_alteration)
{
	union semun arg = { 0 };
	struct sembuf op = { .sem_num = 0, .sem_op = 0, .sem_flg = 0 };
	int semid = TEST_SUCC(create_sem_set(2));

	TEST_RES(semctl(semid, 0, GETPID, arg), _ret == 0);
	TEST_SUCC(semop(semid, &op, 1));
	TEST_RES(semctl(semid, 0, GETPID, arg), _ret == getpid());

	op.sem_num = 1;
	op.sem_op = 1;
	TEST_RES(semctl(semid, 1, GETPID, arg), _ret == 0);
	TEST_SUCC(semop(semid, &op, 1));
	TEST_RES(semctl(semid, 1, GETPID, arg), _ret == getpid());

	TEST_SUCC(remove_sem_set(semid));
}
END_TEST()

FN_TEST(semctl_waiter_counts_report_blocking_semnum)
{
	struct timed_semop_args wait = {
		.nops = 2,
		.timeout_ms = LONG_TIMEOUT_MS,
		.ops = { { .sem_num = 0, .sem_op = 0, .sem_flg = 0 },
			 { .sem_num = 1, .sem_op = -1, .sem_flg = 0 } },
	};
	pthread_t thread;
	int semid = TEST_SUCC(create_sem_set(2));

	wait.semid = semid;
	TEST_SUCC(set_sem_val(semid, 0, 1));

	TEST_SUCC(start_timed_semop_thread(&thread, &wait));
	sleep_ms(SETTLE_MS);

	// We're waiting at the first semaphore for zeros.
	TEST_RES(get_sem_ncnt(semid, 0), _ret == 0);
	TEST_RES(get_sem_zcnt(semid, 0), _ret == 1);
	TEST_RES(get_sem_ncnt(semid, 1), _ret == 0);
	TEST_RES(get_sem_zcnt(semid, 1), _ret == 0);

	TEST_SUCC(set_sem_val(semid, 0, 0));
	sleep_ms(SETTLE_MS);

	// We're waiting at the second semaphore for decreasing.
	TEST_RES(get_sem_ncnt(semid, 0), _ret == 0);
	TEST_RES(get_sem_zcnt(semid, 0), _ret == 0);
	TEST_RES(get_sem_ncnt(semid, 1), _ret == 1);
	TEST_RES(get_sem_zcnt(semid, 1), _ret == 0);

	TEST_SUCC(set_sem_val(semid, 1, 1));
	TEST_RES(join_timed_semop_thread(thread, &wait), _ret == 0);

	// There are no waiters.
	TEST_RES(get_sem_ncnt(semid, 0), _ret == 0);
	TEST_RES(get_sem_zcnt(semid, 0), _ret == 0);
	TEST_RES(get_sem_ncnt(semid, 1), _ret == 0);
	TEST_RES(get_sem_zcnt(semid, 1), _ret == 0);

	TEST_RES(get_sem_val(semid, 0), _ret == 0);
	TEST_RES(get_sem_val(semid, 1), _ret == 0);

	TEST_SUCC(remove_sem_set(semid));
}
END_TEST()

FN_TEST(semtimedop_reports_error_after_blocking)
{
	struct timed_semop_args wait = {
		.nops = 2,
		.timeout_ms = LONG_TIMEOUT_MS,
		.ops = { { .sem_num = 0, .sem_op = -1, .sem_flg = 0 },
			 { .sem_num = 1, .sem_op = 1, .sem_flg = 0 } },
	};
	pthread_t thread;
	int semid = TEST_SUCC(create_sem_set(2));

	TEST_SUCC(set_sem_val(semid, 1, SEMVMX));

	wait.semid = semid;
	TEST_SUCC(start_timed_semop_thread(&thread, &wait));
	sleep_ms(SETTLE_MS);
	TEST_RES(get_sem_ncnt(semid, 0), _ret == 1);

	TEST_SUCC(set_sem_val(semid, 0, 1));
	TEST_RES(join_timed_semop_thread(thread, &wait), _ret == ERANGE);
	TEST_RES(get_sem_val(semid, 0), _ret == 1);
	TEST_RES(get_sem_val(semid, 1), _ret == SEMVMX);

	TEST_SUCC(remove_sem_set(semid));
}
END_TEST()

FN_TEST(semop_updates_otime_after_blocking)
{
	union semun arg = { 0 };
	struct timed_semop_args wait = {
		.nops = 1,
		.timeout_ms = LONG_TIMEOUT_MS,
		.ops = { { .sem_num = 0, .sem_op = -1, .sem_flg = 0 } }
	};
	pthread_t thread;
	struct semid_ds semid_ds;
	int semid = TEST_SUCC(create_sem_set(1));

	wait.semid = semid;
	TEST_SUCC(start_timed_semop_thread(&thread, &wait));
	sleep_ms(SETTLE_MS);
	TEST_RES(get_sem_ncnt(semid, 0), _ret == 1);

	arg.buf = &semid_ds;
	TEST_RES(semctl(semid, 0, IPC_STAT, arg), semid_ds.sem_otime == 0);

	TEST_SUCC(set_sem_val(semid, 0, 1));
	TEST_RES(join_timed_semop_thread(thread, &wait), _ret == 0);
	TEST_RES(get_sem_val(semid, 0), _ret == 0);

	arg.buf = &semid_ds;
	TEST_RES(semctl(semid, 0, IPC_STAT, arg), semid_ds.sem_otime != 0);

	TEST_SUCC(remove_sem_set(semid));
}
END_TEST()
