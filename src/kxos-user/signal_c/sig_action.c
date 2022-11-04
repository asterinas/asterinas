#include <stdio.h>
#include <signal.h> 
#include <unistd.h>
#include <errno.h>
#include <sys/wait.h>

int sigchld = 0;

void proc_exit() {
    sigchld = sigchld + 1;
}

int main() {
    signal(SIGCHLD, proc_exit);
    printf("Run a parent process has pid = %d\n", getpid());
    fflush(stdout);
    int pid = fork();
    if(pid == 0) {
        // child process
        printf("create a new proces successfully (pid = %d)\n", getpid());
        fflush(stdout);
    } else {
        // parent process
        wait(NULL);
        printf("sigchld = %d\n", sigchld);
        fflush(stdout);
    }
    return 0;
}