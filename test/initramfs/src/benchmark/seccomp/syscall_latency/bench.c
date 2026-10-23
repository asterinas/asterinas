// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <errno.h>
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static uint64_t now_ns(void) {
  struct timespec ts;
  if (clock_gettime(CLOCK_MONOTONIC, &ts))
    exit(1);
  return (uint64_t)ts.tv_sec * 1000000000 + ts.tv_nsec;
}

static void measure(unsigned layers, unsigned len, unsigned parameters,
                    unsigned iterations, unsigned warmup) {
  struct sock_filter *code = calloc(len, sizeof(*code));
  if (!code || prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0))
    exit(1);
  for (unsigned i = 0; i + 1 < len; ++i)
    code[i] = (struct sock_filter)BPF_STMT(BPF_LD | BPF_IMM, 0);
  if (parameters) {
    code[0] = (struct sock_filter)BPF_STMT(BPF_LD | BPF_W | BPF_ABS, 16);
    code[1] = (struct sock_filter)BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K,
                                           0x12345678, 0, 1);
    code[2] = (struct sock_filter)BPF_STMT(BPF_RET | BPF_K,
                                           SECCOMP_RET_ERRNO | EACCES);
  }
  code[len - 1] =
      (struct sock_filter)BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW);
  struct sock_fprog prog = {.len = len, .filter = code};
  uint64_t start = now_ns();
  for (unsigned i = 0; i < layers; ++i) {
    if (syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog)) {
      perror("seccomp");
      exit(1);
    }
  }
  uint64_t installation_ns = now_ns() - start;
  for (unsigned i = 0; i < warmup; ++i)
    syscall(SYS_getppid, 0, 0, 0, 0, 0, 0);
  start = now_ns();
  for (unsigned i = 0; i < iterations; ++i)
    syscall(SYS_getppid, 0, 0, 0, 0, 0, 0);
  uint64_t elapsed = now_ns() - start;
  printf("%u,%u,%u,%llu,%.3f\n", layers, len, parameters,
         (unsigned long long)installation_ns, (double)elapsed / iterations);
  free(code);
}

int main(int argc, char **argv) {
  unsigned iterations = argc > 1 ? strtoul(argv[1], NULL, 10) : 100000;
  unsigned warmup = argc > 2 ? strtoul(argv[2], NULL, 10) : 10000;
  if (!iterations)
    return 1;
  puts("layers,instructions,parameter_policy,install_ns,syscall_ns");
  fflush(NULL);
  const unsigned lengths[] = {4, 64, 512};
  const unsigned depths[] = {0, 1, 4, 16};
  for (unsigned len = 0; len < 3; ++len) {
    for (unsigned depth = 0; depth < 4; ++depth) {
      for (unsigned parameters = 0; parameters < 2; ++parameters) {
        for (unsigned repeat = 0; repeat < 5; ++repeat) {
          pid_t pid = fork();
          if (pid < 0)
            return 1;
          if (pid == 0) {
            measure(depths[depth], lengths[len], parameters, iterations,
                    warmup);
            exit(0);
          }
          int status;
          if (waitpid(pid, &status, 0) != pid || !WIFEXITED(status) ||
              WEXITSTATUS(status))
            return 1;
        }
      }
    }
  }
  return 0;
}
