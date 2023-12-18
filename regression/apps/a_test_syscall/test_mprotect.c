#include <stdio.h>
#include <stdint.h>
#include <unistd.h>
#include <sys/syscall.h> 

#ifndef SYS_mprotect 
#error SYS_mprotect unavailable on this system
#endif
int main(){
    uintptr_t addr = 0xffffffffffffffff;  
    size_t len = 10;            
    uint64_t perms = 0xdeadbeef;         
    long result = syscall(SYS_mprotect, addr, len, perms);
    if (result == 0) {
        printf("syscall succeeded\n");
    } else {
        perror("syscall failed");
    }

    return 0;
}