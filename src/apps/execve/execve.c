#include <stdio.h>
#include <unistd.h>

int main() {
    char* argv[] = { "argv1", "argv2", NULL };
    char* envp[] = { "home=/", "version=1.1", NULL };
    printf("Execve a new file ./hello:\n");
    // flush the stdout content to ensure the content print to console
    fflush(stdout);
    execve("./hello", argv, envp);
    printf("Should not print\n");
    fflush(stdout);
    return 0;
}