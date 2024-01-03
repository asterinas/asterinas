// SPDX-License-Identifier: MPL-2.0

// This test file is from occlum pthread test.

#include <sys/types.h>
#include <pthread.h>
#include <stdio.h>
#include <errno.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/syscall.h>
#ifndef SYS_gettid
#error "SYS_gettid unavailable on this system"
#endif

#define gettid() ((pid_t)syscall(SYS_gettid))

// ============================================================================
// Helper functions
// ============================================================================

#define THROW_ERROR(fmt, ...)   do { \
    printf("\t\tERROR:" fmt " in func %s at line %d of file %s with errno %d: %s\n", \
    ##__VA_ARGS__, __func__, __LINE__, __FILE__, errno, strerror(errno)); \
    return -1; \
} while (0)

// ============================================================================
// Helper macros
// ============================================================================


#define NTHREADS                (3)
#define STACK_SIZE              (8 * 1024)

// ============================================================================
// The test case of concurrent counter
// ============================================================================

#define LOCAL_COUNT             (1000UL)
#define EXPECTED_GLOBAL_COUNT   (LOCAL_COUNT * NTHREADS)

struct thread_arg {
    int                         ti;
    long                        local_count;
    volatile unsigned long     *global_count;
    pthread_mutex_t            *mutex;
};

static void *thread_func(void *_arg) {
    struct thread_arg *arg = _arg;
    for (long i = 0; i < arg->local_count; i++) {
        pthread_mutex_lock(arg->mutex);
        (*arg->global_count)++;
        pthread_mutex_unlock(arg->mutex);
    }
    return NULL;
}

static int test_mutex_with_concurrent_counter(void) {
    /*
     * Multiple threads are to increase a global counter concurrently
     */
    volatile unsigned long global_count = 0;
    pthread_t threads[NTHREADS];
    struct thread_arg thread_args[NTHREADS];
    /*
     * Protect the counter with a mutex
     */
    pthread_mutex_t mutex;
    pthread_mutex_init(&mutex, NULL);
    /*
     * Start the threads
     */
    for (int ti = 0; ti < NTHREADS; ti++) {
        struct thread_arg *thread_arg = &thread_args[ti];
        thread_arg->ti = ti;
        thread_arg->local_count = LOCAL_COUNT;
        thread_arg->global_count = &global_count;
        thread_arg->mutex = &mutex;

        if (pthread_create(&threads[ti], NULL, thread_func, thread_arg) < 0) {
            printf("ERROR: pthread_create failed (ti = %d)\n", ti);
            return -1;
        }
    }
    /*
     * Wait for the threads to finish
     */
    for (int ti = 0; ti < NTHREADS; ti++) {
        if (pthread_join(threads[ti], NULL) < 0) {
            printf("ERROR: pthread_join failed (ti = %d)\n", ti);
            return -1;
        }
    }
    /*
     * Check the correctness of the concurrent counter
     */
    if (global_count != EXPECTED_GLOBAL_COUNT) {
        printf("ERROR: incorrect global_count (actual = %ld, expected = %ld)\n",
               global_count, EXPECTED_GLOBAL_COUNT);
        return -1;
    }

    pthread_mutex_destroy(&mutex);
    return 0;
}

// ============================================================================
// The test case of robust mutex
// ============================================================================

struct thread_robust_arg {
    int                         ti;
    volatile int                *global_count;
    pthread_mutex_t             *mutex;
};

int ret_err = -1;

static void *thread_worker(void *_arg) {
    struct thread_robust_arg *arg = _arg;
    int err = pthread_mutex_lock(arg->mutex);
    if (err == EOWNERDEAD) {
        // The mutex is locked by the thread here, but the state is marked as
        // inconsistent, the thread should call 'pthread_mutex_consistent' to
        // make the mutex consistent again.
        if (pthread_mutex_consistent(arg->mutex) != 0) {
            printf("ERROR: failed to recover the mutex\n");
            return &ret_err;
        }
    } else if (err != 0) {
        printf("ERROR: failed to lock the mutex with error: %d\n", err);
        return &ret_err;
    }
    // Mutex is locked
    (*arg->global_count)++;
    // Wait for other threads to acquire the lock
    sleep(1);
    // Exit without unlocking the mutex, this will makes the mutex in an
    // inconsistent state.
    return NULL;
}

static int test_robust_mutex_with_concurrent_counter(void) {
    volatile int global_count = 0;
    pthread_t threads[NTHREADS];
    struct thread_robust_arg thread_args[NTHREADS];
    // Init robust mutex
    pthread_mutex_t mutex;
    pthread_mutexattr_t attr;
    pthread_mutexattr_init(&attr);
    pthread_mutexattr_setrobust(&attr, PTHREAD_MUTEX_ROBUST);
    pthread_mutex_init(&mutex, &attr);
    // Start the threads
    for (int ti = 0; ti < NTHREADS; ti++) {
        struct thread_robust_arg *thread_arg = &thread_args[ti];
        thread_arg->ti = ti;
        thread_arg->global_count = &global_count;
        thread_arg->mutex = &mutex;

        if (pthread_create(&threads[ti], NULL, thread_worker, thread_arg) < 0) {
            THROW_ERROR("pthread_create failed (ti = %d)", ti);
        }
    }
    // Wait for the threads to finish
    for (int ti = 0; ti < NTHREADS; ti++) {
        int *ret_val;
        if (pthread_join(threads[ti], (void **)&ret_val) < 0) {
            THROW_ERROR("pthread_join failed (ti = %d)", ti);
        }
        // printf("Thread %d joined\n", ti);
        // fflush(stdout);
        if (ret_val && *ret_val != 0) {
            THROW_ERROR("run thread failed (ti = %d) with return val: %d", ti, *ret_val);
        }
    }
    // printf("Thread all exited.\n");
    // fflush(stdout);
    // Check the result
    if (global_count != NTHREADS) {
        THROW_ERROR("incorrect global_count (actual = %d, expected = %d)", global_count,
                    NTHREADS);
    }

    pthread_mutex_destroy(&mutex);
    return 0;
}

// ============================================================================
// The test case of waiting condition variable
// ============================================================================

#define WAIT_ROUND          (10)

struct thread_cond_arg {
    int                         ti;
    volatile unsigned int      *val;
    volatile int               *exit_thread_count;
    pthread_cond_t             *cond_val;
    pthread_mutex_t            *mutex;
};

static void *thread_cond_wait(void *_arg) {
    struct thread_cond_arg *arg = _arg;
    printf("Thread #%d: start to wait on condition variable.\n", arg->ti);
    fflush(stdout);
    for (unsigned int i = 0; i < WAIT_ROUND; ++i) {
        int tid = gettid();
        printf("WAIT ROUND: %d, tid = %d\n", i, tid);
        fflush(stdout);
        pthread_mutex_lock(arg->mutex);
        printf("pthread mutex lock: tid = %d\n", tid);
        fflush(stdout);
        while (*(arg->val) == 0) {
            pthread_cond_wait(arg->cond_val, arg->mutex);
            printf("pthread cond wait: tid = %d\n", tid);
            fflush(stdout);
        }
        pthread_mutex_unlock(arg->mutex);
    }
    (*arg->exit_thread_count)++;
    printf("Thread #%d: exited.\n", arg->ti);
    fflush(stdout);
    return NULL;
}

static int test_mutex_with_cond_wait(void) {
    volatile unsigned int val = 0;
    volatile int exit_thread_count = 0;
    pthread_t threads[NTHREADS];
    struct thread_cond_arg thread_args[NTHREADS];
    pthread_cond_t cond_val = PTHREAD_COND_INITIALIZER;
    pthread_mutex_t mutex = PTHREAD_MUTEX_INITIALIZER;
    /*
     * Start the threads waiting on the condition variable
     */
    for (int ti = 0; ti < NTHREADS; ti++) {
        struct thread_cond_arg *thread_arg = &thread_args[ti];
        thread_arg->ti = ti;
        thread_arg->val = &val;
        thread_arg->exit_thread_count = &exit_thread_count;
        thread_arg->cond_val = &cond_val;
        thread_arg->mutex = &mutex;

        if (pthread_create(&threads[ti], NULL, thread_cond_wait, thread_arg) < 0) {
            printf("ERROR: pthread_create failed (ti = %d)\n", ti);
            return -1;
        }
    }
    /*
     * Unblock all threads currently waiting on the condition variable
     */
    while (exit_thread_count < NTHREADS) {
        pthread_mutex_lock(&mutex);
        val = 1;
        pthread_cond_broadcast(&cond_val);
        pthread_mutex_unlock(&mutex);

        pthread_mutex_lock(&mutex);
        val = 0;
        pthread_mutex_unlock(&mutex);
    }
    /*
     * Wait for the threads to finish
     */
    for (int ti = 0; ti < NTHREADS; ti++) {
        if (pthread_join(threads[ti], NULL) < 0) {
            printf("ERROR: pthread_join failed (ti = %d)\n", ti);
            return -1;
        }
    }
    return 0;
}

int main() {
    test_mutex_with_concurrent_counter();
    test_robust_mutex_with_concurrent_counter();
    // test_mutex_with_cond_wait();
    return 0;
}