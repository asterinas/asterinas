#include <stdio.h>
#include <unistd.h>

int main() {
    printf("before fork\n");
    fflush(stdout);
    if(fork() == 0) {
        printf("after fork: Hello from parent\n");
    } else {
        printf("after fork: Hello from child\n");   
    }
    fflush(stdout);
    return 0;
}