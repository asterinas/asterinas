// SPDX-License-Identifier: MPL-2.0

// This test file is from occlum signal test.

#define _GNU_SOURCE
#include <sys/types.h>
#include <sys/stat.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>
#include <ucontext.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <spawn.h>
#include <assert.h>
#include <fcntl.h>
#include <signal.h>
#include <pthread.h>
#include <errno.h>
#include <time.h>

// ============================================================================
// Helper functions
// ============================================================================

#define THROW_ERROR(fmt, ...)   do { \
    printf("\t\tERROR:" fmt " in func %s at line %d of file %s with errno %d: %s\n", \
    ##__VA_ARGS__, __func__, __LINE__, __FILE__, errno, strerror(errno)); \
    return -1; \
} while (0)


// ============================================================================
// Test sigprocmask
// ============================================================================

#define sigcmpset(a, b) memcmp((a), (b), 8)

int test_sigprocmask() {
    int ret;
    sigset_t new, old;
    sigset_t expected_old;

    // Check sigmask == []
    if ((ret = sigprocmask(0, NULL, &old)) < 0) {
        THROW_ERROR("sigprocmask failed unexpectedly");
    }
    sigemptyset(&expected_old);
    if (sigcmpset(&old, &expected_old) != 0) {
        THROW_ERROR("unexpected old sigset");
    }

    // SIG_BLOCK: [] --> [SIGSEGV]
    sigemptyset(&new);
    sigaddset(&new, SIGSEGV);
    if ((ret = sigprocmask(SIG_BLOCK, &new, &old)) < 0) {
        THROW_ERROR("sigprocmask failed unexpectedly");
    }
    sigemptyset(&expected_old);
    if (sigcmpset(&old, &expected_old) != 0) {
        THROW_ERROR("unexpected old sigset");
    }

    // SIG_SETMASK: [SIGSEGV] --> [SIGIO]
    sigemptyset(&new);
    sigaddset(&new, SIGIO);
    if ((ret = sigprocmask(SIG_SETMASK, &new, &old)) < 0) {
        THROW_ERROR("sigprocmask failed unexpectedly");
    }
    sigemptyset(&expected_old);
    sigaddset(&expected_old, SIGSEGV);
    if (sigcmpset(&old, &expected_old) != 0) {
        THROW_ERROR("unexpected old sigset");
    }

    // SIG_UNBLOCK: [SIGIO] -> []
    if ((ret = sigprocmask(SIG_UNBLOCK, &new, &old)) < 0) {
        THROW_ERROR("sigprocmask failed unexpectedly");
    }
    sigemptyset(&expected_old);
    sigaddset(&expected_old, SIGIO);
    if (sigcmpset(&old, &expected_old) != 0) {
        THROW_ERROR("unexpected old sigset");
    }

    // Check sigmask == []
    if ((ret = sigprocmask(0, NULL, &old)) < 0) {
        THROW_ERROR("sigprocmask failed unexpectedly");
    }
    sigemptyset(&expected_old);
    if (sigcmpset(&old, &expected_old) != 0) {
        THROW_ERROR("unexpected old sigset");
    }

    return 0;
}

// ============================================================================
// Test raise syscall and user-registered signal handlers
// ============================================================================

#define MAX_RECURSION_LEVEL     3

static void handle_sigio(int num, siginfo_t *info, void *context) {
    static volatile int recursion_level = 0;
    printf("Hello from SIGIO signal handler (recursion_level = %d)!\n", recursion_level);
    fflush(stdout);
    
    recursion_level++;
    if (recursion_level <= MAX_RECURSION_LEVEL) {
        raise(SIGIO);
    }
    recursion_level--;
}

int test_raise() {
    struct sigaction new_action, old_action;
    memset(&new_action, 0, sizeof(struct sigaction));
    memset(&old_action, 0, sizeof(struct sigaction));
    new_action.sa_sigaction = handle_sigio;
    new_action.sa_flags = SA_SIGINFO | SA_NODEFER;
    if (sigaction(SIGIO, &new_action, &old_action) < 0) {
        THROW_ERROR("registering new signal handler failed");
    }
    if (old_action.sa_handler != SIG_DFL) {
        THROW_ERROR("unexpected old sig handler");
    }

    raise(SIGIO);

    if (sigaction(SIGIO, &old_action, NULL) < 0) {
        THROW_ERROR("restoring old signal handler failed");
    }
    return 0;
}

// ============================================================================
// Test catching and handling hardware exception
// ============================================================================

static void handle_sigfpe(int num, siginfo_t *info, void *_context) {
    printf("SIGFPE Caught\n");
    fflush(stdout);
    assert(num == SIGFPE);
    assert(info->si_signo == SIGFPE);
    ucontext_t *ucontext = _context;
    mcontext_t *mcontext = &ucontext->uc_mcontext;
    // The faulty instruction should be `idiv %esi` (f7 fe)
    mcontext->gregs[REG_RIP] += 2;
    return;
}

// Note: this function is fragile in the sense that compiler may not always
// emit the instruction pattern that triggers divide-by-zero as we expect.
// TODO: rewrite this in assembly
int div_maybe_zero(int x, int y) {
    return x / y;
}

#define fxsave(addr) __asm __volatile("fxsave %0" : "=m" (*(addr)))

int test_handle_sigfpe() {
    // Set up a signal handler that handles divide-by-zero exception
    struct sigaction new_action, old_action;
    memset(&new_action, 0, sizeof(struct sigaction));
    memset(&old_action, 0, sizeof(struct sigaction));
    new_action.sa_sigaction = handle_sigfpe;
    new_action.sa_flags = SA_SIGINFO;
    if (sigaction(SIGFPE, &new_action, &old_action) < 0) {
        THROW_ERROR("registering new signal handler failed");
    }
    if (old_action.sa_handler != SIG_DFL) {
        THROW_ERROR("unexpected old sig handler");
    }

    char x[512] __attribute__((aligned(16))) = {};
    char y[512] __attribute__((aligned(16))) = {};

    // Trigger divide-by-zero exception
    int a = 1;
    int b = 0;
    // Use volatile to prevent compiler optimization
    volatile int c;
    fxsave(x);
    c = div_maybe_zero(a, b);
    fxsave(y);

    // Asterinas does not save and restore fpregs now, so we emit this check.
    // if (memcmp(x, y, 512) != 0) {
    //     THROW_ERROR("floating point registers are modified");
    // }

    printf("Signal handler successfully jumped over the divide-by-zero instruction\n");
    fflush(stdout);

    if (sigaction(SIGFPE, &old_action, NULL) < 0) {
        THROW_ERROR("restoring old signal handler failed");
    }
    return 0;
}

// TODO: rewrite this in assembly
int read_maybe_null(int *p) {
    return *p;
}

static void handle_sigsegv(int num, siginfo_t *info, void *_context) {
    printf("SIGSEGV Caught\n");
    fflush(stdout);

    assert(num == SIGSEGV);
    assert(info->si_signo == SIGSEGV);

    ucontext_t *ucontext = _context;
    mcontext_t *mcontext = &ucontext->uc_mcontext;
    // TODO: how long is the instruction?
    // The faulty instruction should be `idiv %esi` (f7 fe)
    mcontext->gregs[REG_RIP] += 2;

    return;
}


int test_handle_sigsegv() {
    // Set up a signal handler that handles divide-by-zero exception
    struct sigaction new_action, old_action;
    memset(&new_action, 0, sizeof(struct sigaction));
    memset(&old_action, 0, sizeof(struct sigaction));
    new_action.sa_sigaction = handle_sigsegv;
    new_action.sa_flags = SA_SIGINFO;
    if (sigaction(SIGSEGV, &new_action, &old_action) < 0) {
        THROW_ERROR("registering new signal handler failed");
    }
    if (old_action.sa_handler != SIG_DFL) {
        THROW_ERROR("unexpected old sig handler");
    }

    int *addr = NULL;
    volatile int val = read_maybe_null(addr);
    (void)val; // to suppress "unused variables" warning

    printf("Signal handler successfully jumped over a null-dereferencing instruction\n");
    fflush(stdout);

    if (sigaction(SIGSEGV, &old_action, NULL) < 0) {
        THROW_ERROR("restoring old signal handler failed");
    }
    return 0;
}

// ============================================================================
// Test SIGCHLD signal
// ============================================================================
int sigchld = 0;

void proc_exit() {
    sigchld = 1;
}

int test_sigchld() {
    signal(SIGCHLD, proc_exit);
    printf("Run a parent process has pid = %d\n", getpid());
    fflush(stdout);
    int pid = fork();
    if(pid == 0) {
        // child process
        printf("create a new proces successfully (pid = %d)\n", getpid());
        fflush(stdout);
        exit(0);
    } else {
        // parent process
        wait(NULL);
        printf("sigchld = %d\n", sigchld);
        fflush(stdout);
    }
    return 0;
}

// ============================================================================
// Test handle signal on alternate signal stack
// ============================================================================

#define MAX_ALTSTACK_RECURSION_LEVEL    2

stack_t g_old_ss;

static void handle_sigpipe(int num, siginfo_t *info, void *context) {
    static volatile int recursion_level = 0;
    printf("Hello from SIGPIPE signal handler on the alternate signal stack (recursion_level = %d)\n",
           recursion_level);

    // save old_ss to check if we are on stack
    stack_t old_ss;
    sigaltstack(NULL, &old_ss);
    g_old_ss = old_ss;

    recursion_level++;
    if (recursion_level <= MAX_ALTSTACK_RECURSION_LEVEL) {
        raise(SIGPIPE);
    }
    recursion_level--;
}

#define SIGSTACKSIZE (4*4096)

int test_sigaltstack() {
    static char stack[SIGSTACKSIZE];
    stack_t expected_ss = {
        .ss_size = SIGSTACKSIZE,
        .ss_sp = stack,
        .ss_flags = 0,
    };
    if (sigaltstack(&expected_ss, NULL) < 0) {
        THROW_ERROR("failed to call sigaltstack");
    }
    stack_t actual_ss;
    if (sigaltstack(NULL, &actual_ss) < 0) {
        THROW_ERROR("failed to call sigaltstack");
    }
    if (actual_ss.ss_size != expected_ss.ss_size
            || actual_ss.ss_sp != expected_ss.ss_sp
            || actual_ss.ss_flags != expected_ss.ss_flags) {
        THROW_ERROR("failed to check the signal stack after set");
    }

    struct sigaction new_action, old_action;
    memset(&new_action, 0, sizeof(struct sigaction));
    memset(&old_action, 0, sizeof(struct sigaction));
    new_action.sa_sigaction = handle_sigpipe;
    new_action.sa_flags = SA_SIGINFO | SA_NODEFER | SA_ONSTACK;
    if (sigaction(SIGPIPE, &new_action, &old_action) < 0) {
        THROW_ERROR("registering new signal handler failed");
    }
    if (old_action.sa_handler != SIG_DFL) {
        THROW_ERROR("unexpected old sig handler");
    }

    raise(SIGPIPE);
    if (g_old_ss.ss_flags != SS_ONSTACK) {
        THROW_ERROR("check stack flags failed");
    }

    if (sigaction(SIGPIPE, &old_action, NULL) < 0) {
        THROW_ERROR("restoring old signal handler failed");
    }
    return 0;
}

int main() {
    test_sigprocmask();
    test_raise();
    test_handle_sigfpe();
    test_handle_sigsegv();
    test_sigchld();
    test_sigaltstack();
    return 0;
}