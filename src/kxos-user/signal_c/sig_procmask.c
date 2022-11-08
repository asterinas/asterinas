/// This code is from CSAPP
/// We use this codes to test sigprocmask
#include <stdio.h>
#include <signal.h> 

int main() {
    sigset_t mask, prev_mask;
    sigemptyset(&mask);
    sigaddset(&mask, SIGINT);
    sigaddset(&mask, SIGCHLD);

    /* Block SIGINT and save previous blocked set */
    sigprocmask(SIG_BLOCK, &mask, &prev_mask);
    
    // Code region that will not be interrupted by SIGINT and SIGCHILD
    /* Restore previous blocked set, unblocking SIGINT */
    sigprocmask(SIG_SETMASK, &prev_mask, NULL);
    return 0;
}