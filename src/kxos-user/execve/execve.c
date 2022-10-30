#include <stdio.h>
#include <unistd.h>

int main() {
    char* argv[] = { NULL };
    char* envp[] = { NULL };
    printf("Execve a new file ./hello:\n");
    // flust the stdout content to ensure the content print to console
    fflush(stdout);
    execve("./hello", argv, envp);
    printf("Should not print\n");
    return 0;
}