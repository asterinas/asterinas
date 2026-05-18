// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>

__attribute__((noinline)) void hello_world(int x, int *heap_value)
{
    printf("Hello, World %d!\n", x);
    fflush(stdout);
}

int main(void)
{
    int *heap_value = malloc(sizeof(*heap_value));
    if (heap_value == NULL)
        return 2;

    *heap_value = 4321;
    for (int i = 0; i < 5; i++)
        hello_world(i, heap_value);

    if (*heap_value != 1234) {
        printf("memory check failed: %d\n", *heap_value);
        free(heap_value);
        return 3;
    }

    printf("memory check passed: %d\n", *heap_value);
    free(heap_value);
    return 0;
}
