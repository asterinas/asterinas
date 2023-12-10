#include <stdio.h>
#include <stdint.h>
#include <unistd.h>
#include <sys/syscall.h> //系统调用所需头文件

#ifndef SYS_munmap //判断语句，这里的SYS_mprotec需要被定义中断号

#error SYS_munmap unavailable on this system
#endif
int main(){
    uintptr_t addr = 0xffffffffffffffff;  
    size_t len = 10;            
    uint64_t perms = 0xdeadbeef;         
    // 触发软中断，传递参数
    long result = syscall(SYS_munmap, addr, len, perms);
    if (result == 0) {
        printf("syscall succeeded\n");
    } else {
        perror("syscall failed");
    }

    return 0;
}